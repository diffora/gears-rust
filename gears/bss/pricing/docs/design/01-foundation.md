<!-- CONFLUENCE_TITLE: [BSS]: Plan & Price Modeling — Catalog Foundation (shared publish engine) (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md | Upstream: Product & SKU registry, Effective-dating PriceWindows | Downstream: Tariffs, Subscriptions, Rating, Billing, Marketplace | Owners: BSS Product Catalog team -->

# DESIGN — Catalog Foundation (shared publish engine) (Slice 1)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-foundation`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles and Constraints](#2-principles-and-constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions and Sequences](#36-interactions-and-sequences)
  - [3.7 Database Schemas and Tables](#37-database-schemas-and-tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional Context](#4-additional-context)
  - [4.1 Canonical Scope Key (normative)](#41-canonical-scope-key-normative)
  - [4.2 Publish-Through-The-Engine Contract (normative)](#42-publish-through-the-engine-contract-normative)
  - [4.3 Immutability and Change Mechanisms (normative)](#43-immutability-and-change-mechanisms-normative)
  - [4.4 Read Model and pricingSnapshotRef (normative)](#44-read-model-and-pricingsnapshotref-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The Catalog Foundation is the shared publish engine that every Plan & Price Modeling
capability builds on. It owns the `Plan`/`Price` entity model, the **canonical scope key**,
the draft→publish state machine, the **fail-closed validation pipeline** framework,
append-only published-row history with versioning/supersession, the **read-model projection**
and `pricingSnapshotRef` stamping, and the **frozen event fan-out** plus the
`CatalogVersion`-increment request to the registry. It owns **no capability policy**: what a
billing cycle means, how a tier band validates, what a bundle is — all live in a handler slice
(plan-definition, price-structure, and the rest), each of which authors draft state, registers
its validation rules and its read-model fields, and **publishes through** this Foundation's
API under the invariants defined here.

The catalog's contract is the mirror image of the sibling Billing Ledger's *post through the
engine*: it is **publish through the engine**. Every state change reaches production one way —
author draft → run the aggregate fail-closed validation pipeline → freeze a complete read
model + `pricingSnapshotRef` → emit the frozen event set → request a `CatalogVersion`. There
is no side door that mutates published state, no consumer that reads draft, and no default
substituted for an absent field (absence must have failed publish). This keeps the
correctness-critical publish/immutability/determinism core small and auditable while letting
each pricing capability evolve independently ([`../PRD.md`](../PRD.md) §1.1, §2).

The gear is **one deployable modular monolith** (`pricing`) running in two roles — a
synchronous authoring/publish/preview API and a read-model service — over one PostgreSQL
backend. The authoring path is transactional — draft mutation, validation, publish-commit, and event
enqueue commit atomically **at the post-approval publish step** (the Slice 5 approval gate for
material changes sits between the validation pre-check and that commit); the read path is
served from a projected read model for the p95 < 100ms target and **fails closed** (never
stale) on read-model outage.

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-pricing-fr-publish-validation-failclosed` | A single **aggregate fail-closed validation pipeline** runs at publish; slices register rules keyed to the §17.4 rule set; any invalid condition blocks the publish transaction (no `PlanPublished`, no read-model warm). The validation report enumerates every failure so authoring can remediate. |
| `cpt-cf-bss-pricing-fr-published-rows-append-only` | `pricing_price` rows in a published state are append-only: `REVOKE UPDATE, DELETE` from the app role + `BEFORE UPDATE OR DELETE` triggers that RAISE, with a **column whitelist** for the two sanctioned transitions (state-machine `lifecycle_state` flips; monotonic `grandfather_until` tightening — §3.7/§4.3); only never-published `draft` rows are deletable. There is no deletion event to fan out. |
| `cpt-cf-bss-pricing-fr-plan-versioning` | A price/tier change versions the `Plan` and writes **new** immutable `pricing_price` rows; prior rows are retained as history; bound subscriptions continue on their frozen snapshot until renewal or migration. |
| `cpt-cf-bss-pricing-fr-supersession` | Supersession is versioning scoped to **one canonical scope key**: a new immutable row plus opening/closing the corresponding `PriceWindow` (never an in-place mutate, never an overlap), within one `priceEligibility` class and one `chargeKind`. |
| `cpt-cf-bss-pricing-fr-pricing-snapshot` | Publish stamps the catalog-side identifiers sufficient for the manifest `pricingSnapshotRef` (resolved price ids + evaluation-policy version + the **pending** version ref, finalized to the committed `CatalogVersion` on `CatalogVersionPublished`); posted periods never re-query mutable rows; the catalog-side view MUST NOT diverge from the Tariffs composition SoR. |
| `cpt-cf-bss-pricing-fr-consumer-readmodel-resolution` | The projected read model is **monotonic per `CatalogVersion`** (a version is **pin-eligible** only once `CatalogVersionPublished` has fired, **every** subject row it projects is warm-complete — D-101 — **and every earlier version is itself pin-eligible** — D-114: pin-eligibility is a prefix-closed frontier); consumers resolve exact published values with no draft read and no default substitution; a rating run pins one pin-eligible version and the pin never lags the newest pin-eligible version by > 5s. |
| `cpt-cf-bss-pricing-fr-catalogversion-increment` | On every `PlanPublished` the Foundation requests addressability; the registry is the **sole** incrementer and MAY batch approved publishes; `PlanPublished` carries a **pending** ref and the snapshot pins the committed version on `CatalogVersionPublished`. |
| `cpt-cf-bss-pricing-fr-publish-fanout-atomicity` | Post-commit read-model warming retries to the 5s SLO or marks the publish degraded (`PlanPublishDegraded`); no state exposes a rateable-but-incomplete plan; the pre-commit batching delay is governed by the max batching-delay SLO, not by degraded handling. |
| `cpt-cf-bss-pricing-fr-event-contract` | A **frozen event-name set** (`PlanCreated`, `PlanUpdated`, `PlanPublished`, `PlanRetired`, and conditionally `PlanMigrationScheduled`, `PlanPublishDegraded`, `BundleUpdated`, `PriceCreated`, `PriceUpdated`, plus the manifest `PriceWindowScheduled`/`Activated`/`Expired`/`Cancelled` — produced by this gear since the window consolidation, D-03) emitted from a transactional outbox, ordered per `(tenantId, aggregateId)`, at-least-once, carrying correlation/idempotency keys. |
| `cpt-cf-bss-pricing-fr-price-amount-validation` | Amount ≥ 0, valid ISO 4217, precision = the currency's ISO 4217 minor unit; a missing `(currency, region)` row fails closed (no implicit FX). |
| `cpt-cf-bss-pricing-fr-mutation-idempotency` | Plan/Price create/update accept a client idempotency key; a duplicate returns the original outcome without a second mutation. |
| `cpt-cf-bss-pricing-fr-concurrent-edit` | Optimistic concurrency (ETag/row version) rejects a stale submit and a bulk-vs-interactive collision with a conflict; neither change is silently overwritten. |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| Publish → read-model propagation (p95 ≤ 5s) | Publish engine + outbox + read-model warmer | Batched `CatalogVersion` commit; retry-to-SLO warm or `PlanPublishDegraded`; pin never lags newest completed by > 5s | Load test on the publish→warm path; batching-delay SLO **ratified (D-47: p95 ≤ 60s, max 5 min; interactive ≤ 5s)** ([`../PRD.md`](../PRD.md) §15) |
| Read / preview latency (p95 < 100ms per tenant partition) | Read-model projection store | Single indexed, version-pinned read; no evaluation on the read path | APM on read APIs |
| Determinism / reproducibility | Snapshot + append-only history | Complete frozen `pricingSnapshotRef`, monotonic per version, append-only rows | Design + integration test (later-version publish does not alter a prior snapshot) |
| Read-model availability / DR RPO-RTO | Read-model store + topology | Fail-closed on outage (never stale); 99.9% / RPO 5m / RTO 30m | Committed — ratified 2026-07-28 ([`../PRD.md`](../PRD.md) §14) |
| Idempotency-key TTL; plan/tier size caps | Publish engine | Idempotency-dedup store (TTL **24h**, evaluated **at claim time** and **before** the payload-digest comparison — **D-142**, 2026-08-02, found while building the draft-authoring plane: compared the other way round, the first payload to touch a key owns it forever, since nothing deletes a dedup row, and the ratified 24h is then not a bound at all; §3.7 carries the row's two states); publish-time size validation (100 bands/row, 500 rows/plan soft; 366d/24m interval caps) | Committed — ratified 2026-07-28 |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|------------------|
| `cpt-cf-bss-pricing-adr-canonical-scope-key` | The single scope key is `(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)` — the manifest key extended additively so hybrid components and a grandfathered row + its successor are distinct keys with concurrent active windows. |
| `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis` | Multi-generation grandfathering: the additive `cohort` axis (= the cutover instant; `none` on non-grandfathered rows) makes every cutover a **new** generation on its own key; within the grandfathered class Tariffs selects by the cohort of the subscription's pinned price id. |
| `cpt-cf-bss-pricing-adr-pricewindow-consolidation` | The `PriceWindow` machinery is gear-owned (Slice 7): store, state machine, activation job, `PriceWindow*` production; multi-window units are local ACID transactions. |

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-pricing-tech-stack`

```text
Capability slices (modules)   plan-definition · price-structure · currency-tax · governance ·
                              consumer-contracts · pricewindow-linkage · bundles · price-overlays ·
                              advanced-primitives · lifecycle · operator-efficiency
        │  (publish API: authorDraft / validate / publish / projectReadModel / requestVersion)
        ▼
Publish Engine (Foundation)   ScopeKey · DraftStateMachine · ValidationPipeline ·
                              VersioningStore · ReadModelProjector · SnapshotStamper · EventOutbox
        │
        ▼
PostgreSQL                    plan / price (truth + append-only history) · read_model (projection) ·
                              catalog_version_ref · outbox · policy objects · audit store
```

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Presentation | REST authoring/publish/preview + read-model surfaces behind the inbound gateway; RFC 9457 problems; OAuth 2.0; ETag optimistic concurrency | Rust, REST/OpenAPI, inbound API gateway |
| Application | Capability slices author draft state and register rules/read-model fields | Rust modules in the `pricing` monolith |
| Domain | The publish engine, canonical scope key, validation pipeline, versioning/immutability, snapshot contract | Rust; GTS + Rust domain structs |
| Infrastructure | Append-only history, projected read model, audit store, transactional outbox | PostgreSQL (single primary + replicas), SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Foundation owns publish; slices own capability policy

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-foundation-owns-publish-fnd`

No slice defines the scope key, emits an event, or stamps a snapshot; the Foundation defines
no capability semantics. Slices author draft state, register validation rules and read-model
fields, and publish through the Foundation API. Normative: [§4.2](#42-publish-through-the-engine-contract-normative).

#### Fail closed, always

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-fail-closed-fnd`

Any invalid or ambiguous condition blocks the publish transaction; the absence of a required
field is a publish failure, never a downstream default. Consumers never read draft state.

#### Published state is append-only

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-append-only-fnd`

Published `pricing_price` rows are immutable history; change is a new immutable row via
versioning/supersession + `PriceWindow`. Only never-published `draft` rows are deletable.
Normative: [§4.3](#43-immutability-and-change-mechanisms-normative).

### 2.2 Constraints

#### Money is ISO 4217 minor units

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-money-fnd`

Amounts are stored as integer minor units at the currency's ISO 4217 scale (0 for JPY/KRW, 2
default, 3 for BHD/KWD/OMR); a flat 2-decimal cap MUST NOT be assumed; amounts are `≥ 0`
(a negative amount is rejected — `AMOUNT_NEGATIVE`); a code that is not valid ISO 4217 is
rejected (`CURRENCY_INVALID`); no implicit FX — a missing `(currency, region)` row fails closed.

#### Author-driven mutation; UTC time

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-author-driven-fnd`

The catalog mutates state only in response to explicit authoring/publish/lifecycle calls; it
does not self-originate rows. All effective dating, window boundaries, `grandfatherUntil`,
`availableFrom`/`availableTo`, and anchor math are UTC.

**UTC at millisecond resolution (normative, D-144, 2026-08-02, found while building the
draft-authoring plane).** Every instant this gear **authors, carries in a contract field,
publishes or compares** is quantized to the millisecond: cutover instants and the `cohort` axis
derived from them (§4.1), window `effectiveFrom`/`effectiveTo`, `grandfatherUntil`,
`availableFrom`/`availableTo`, and the D-88 changeover instant. An authored instant carrying
finer precision **fails validation** (`TIMESTAMP_PRECISION_EXCEEDED`, 422) rather than being
truncated. The line above fixed the zone and left the resolution open, which is not a spelling
gap: `cohort` is a scope-key axis matched for **equality**, and D-126's bootstrap case compares
it against a window `effectiveTo` produced by a different code path in a different gear — two
instants denoting the same moment at different resolutions are not equal, so an unquantized axis
makes a generation unfindable by exactly the subscribers grandfathering exists to protect.
Rejected: **truncating at the boundary**, which is what an unstated quantum degenerates into. It
silently moves the instant a scope-key axis, a window bound and an approval-time floor are all
derived from, and a truncating producer agrees with a non-truncating consumer until the day they
do not, with no failure in between — the same posture the money-side sibling `PRECISION_EXCEEDED`
(§3.3) already takes, for the same reason. Storage bookkeeping no operator authors — `created_at`,
audit-chain and outbox timestamps — is outside the rule: it enters no contract field and is
compared with nothing. The published field set is unchanged; **joint with Rating**, the owed
D-126 cohort-bootstrap adoption gains one clause, that the comparison is at this quantum.

#### AuthZ gate before the repository

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-authz-gate-fnd`

Every ctx-bearing service path calls the shared PEP `access_scope` gate
(`authz_resolver_sdk`) with its catalogued `(resource_type, action)` pair **before** touching
the repository; the PDP-compiled `AccessScope` is the SQL filter SecureORM binds (reads) and
the write-target membership assertion (writes). The resource/action catalog, the
endpoint mapping, and the role matrix are normative in the governance slice
([`05-governance.md`](./05-governance.md) §AuthZ Resource and Action Catalog); labels are
GTS ids `gts.cf.bss.pricing.<noun>.v1~` outside `gts.cf.resources.*`, registered as stub
type-schemas at gear init.

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-domain-model-fnd`

The Foundation owns four core aggregates; capability slices extend them with their own fields
and child tables (phases, add-on rules, bundles, price overlays) without redefining the core.

- **`Plan`** — binds a **published** `skuId` to a billing cycle, a mandatory `PlanTier`, optional plan phases and composition rules, and optional `availableFrom`/`availableTo` purchasability dates. Carries a lifecycle state per revision row (`draft` → `published` → `superseded` | `retired`, plus the terminal `abandoned` a discarded draft revision takes — D-145; the `superseded` flip is D-90's plan-revision analogue of the price rows' flip-at-commit) and a `revision` that is an **identity**, minted `max + 1` and never re-minted (D-145 — §4.3). Capability meaning of the cycle/composition fields is owned by the plan-definition slice.
- **`Price`** — a price row on the **canonical scope key** (§4.1) with an amount (ISO 4217 minor units), `modelKind`, tier bands, evaluation-policy fields, `taxInclusive`, lifecycle metadata (`priceEligibility`, optional `grandfatherUntil`), and a supersession pointer to the row it replaces within its scope key. Published rows are append-only; a prior row is retained as history.
- **`ReadModel`** — the projected, per-`CatalogVersion` frozen view a consumer resolves: `{skuId, planId, priceId}`, model kind, ordered tier bands, evaluation-policy fields, phase→price map, **phase→grant-set map** (per-phase entitlement grant set when authored — D-41; else the plan-level `PlanTier`-driven grant set), billing descriptors, and the consumer contracts (proration/plan-change/entitlement) contributed by their slices. Monotonic per version; never reflects draft state.
- **`pricingSnapshotRef`** — the composite reference (resolved price ids + evaluation-policy version + version ref) whose catalog-side identifiers are stamped at publish with a **pending** version ref and **finalized** to the committed `CatalogVersion` on `CatalogVersionPublished`, immutable thereafter; pinned by consumers (composition SoR: Tariffs).

Supporting Foundation objects: the **tenant policy objects** (approval-threshold policy,
tax-display policy — both fail-safe), the **idempotency-dedup** store, the **transactional
outbox**, and the **audit store** (append-only, actor/before-after/approval trail).

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-foundation-fnd`

The Foundation is a set of in-process components behind one publish API:

- **`ScopeKey`** — constructs and validates the canonical scope key, applies the axis defaults (`priceOverlay = base`, `phase =` the plan's terminal `phase_id`, `priceEligibility = all_subscriptions`, `cohort = none`), and backs the row-uniqueness index.
- **`DraftStateMachine`** — the `draft` → `published` → `superseded` | `retired` transitions (per revision row — D-90), plus the terminal `draft → abandoned` flip a discarded plan draft revision takes (**D-145**, 2026-08-02 — [`02-plan-definition.md`](./02-plan-definition.md) `inst-pl-abandon`). Only `draft` rows are mutable; only never-published `draft` **price** rows are deletable — a plan's open draft revision row is abandoned rather than deleted, so the `revision` number it consumed is never re-minted (§4.3).
- **`ValidationPipeline`** — runs the aggregate fail-closed rule set at publish; slices register rules; a single failure blocks the publish transaction and populates the validation report.
- **`VersioningStore`** — writes new immutable rows on versioning/supersession, retains history, and enforces append-only via role + triggers.
- **`ReadModelProjector`** — materialises the frozen per-version read model and drives warm-completion; fails closed on outage.
- **`SnapshotStamper`** — stamps the catalog-side identifiers sufficient for the manifest `pricingSnapshotRef` (composition SoR: Tariffs) and requests the `CatalogVersion` increment from the registry.
- **`EventOutbox`** — emits the frozen event set transactionally, ordered per `(tenantId, aggregateId)`, at-least-once with dedup keys.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-interface-authoring-publish-fnd`

The **authoring + publish** contract (`cpt-cf-bss-pricing-interface-authoring-publish`):
create/update/clone plans and price rows in `draft`, run fail-closed validation, submit for
approval (two-person rule for material changes), and publish — emitting the frozen event set
and requesting a `CatalogVersion`. Accepts client idempotency keys; enforces optimistic
concurrency via ETag/row version.

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-interface-catalog-read-model-fnd`

The **published read model** contract (`cpt-cf-bss-pricing-interface-catalog-read-model`):
per committed `CatalogVersion`, the complete plan/price read model resolvable via
`pricingSnapshotRef`, monotonic per version, no draft reads, additive-only within a major
version. It additionally publishes the **pin frontier** —
`GET /bss-pricing/v1/catalog-version/frontier` → the tenant's current pin-eligible
`CatalogVersion` + its `advanced_at` (`plan × read`, service identity; D-136, §4.4) — because
pin-eligibility is a version-level, prefix-closed predicate no consumer can evaluate for
itself: this is the value a consumer pins at the start of a resolution/rating run, and the
referent of the ≤ 5s pin-lag rule.

The base-price **preview** (`cpt-cf-bss-pricing-interface-price-preview`) and the external
integration contracts ([`../PRD.md`](../PRD.md) §9.2) are refined in the slices that own their
payloads. Concrete schemas, proto, and **slice-specific** error taxonomies are owned by these
slice designs. Failure modes of the engine itself carry **Foundation-owned** RFC 9457 problem
types, referenced (never redefined) by slices. **Problem responses (RFC 9457):**
`DUPLICATE_SCOPE_KEY` (409 — canonical
scope-key uniqueness), `STALE_VERSION` (409 — ETag/row-version conflict),
`IDEMPOTENCY_PAYLOAD_MISMATCH` (409 — same key, different payload),
`IDEMPOTENCY_KEY_IN_FLIGHT` (409 — this key is claimed and not yet answered; retry, and you
will receive either the stored response or `IDEMPOTENCY_PAYLOAD_MISMATCH`. D-143, 2026-08-02,
found while building the draft-authoring plane, **flagged for veto** — the case arises with no
contract violation by anyone (§3.7), so stating the one-transaction rule normatively cannot
close it, and the surface layer had nothing to answer a client with),
`OPEN_DRAFT_REVISION_EXISTS` (409, naming the open revision — a second draft revision on a plan
that already holds one. D-146, 2026-08-02: a **narrowing** of `LIFECYCLE_FORBIDDEN`, not an
addition beside it, so no refusal ends up with two codes; it is a uniqueness conflict on the
`(plan_id) WHERE lifecycle_state = 'draft'` partial index (§3.7) rather than a state-machine
transition, and the operator's next action is a real one — go and edit that revision — §4.3),
`PLAN_RETIRED_NO_SUCCESSOR` (422 — opening a revision on, or re-publishing, a retired plan.
D-146: the second narrowing, **replacing** the re-publishing clause of the gloss below, because
a retired plan can never publish again and the operator has no next action at all — §4.3),
`PLAN_ABANDONED_NO_SUCCESSOR` (422 — an authoring call naming a plan whose every revision is
`abandoned`: it holds no current revision and no open draft and can acquire neither, so no
further revision — first or successor — will ever open on it, and the plan id is spent.
**D-145 as amended 2026-08-02**: that rule's "a replacement draft opens immediately" holds for
every plan that has published at least once and for no plan that never has, because a first
draft is minted at revision `0` while a successor presupposes a current revision to succeed
from (§4.3). The sibling of `PLAN_RETIRED_NO_SUCCESSOR` and refused for the same structural
reason, but deliberately not merged into it: that code would assert a retirement that never
happened, and a retired plan still has a current revision, a warm delta and a clone route
forward where this one has none. Not `LIFECYCLE_FORBIDDEN` either, by D-146's own line — the
operator's next action here is specific and different: mint a new plan, and stop retrying this
id — §4.3),
`LIFECYCLE_FORBIDDEN` (422 — a transition the state machine does not permit:
mutating a published row's content, superseding a
grandfathered row. Named 2026-08-02 while implementing §4.3 — the transitions
were normative from the start and the refusal had no code, and a refusal a
consumer cannot discriminate is one it must parse prose to understand; **narrowed the same day
by D-146**, which took out the two refusals a consumer must act on differently and left exactly
the refusals that describe no alternative action),
`TIMESTAMP_PRECISION_EXCEEDED` (422 — an authored instant finer than the millisecond quantum
§2.2 fixes. D-144, 2026-08-02: refused rather than truncated, because truncation moves an
instant a scope-key axis is matched on for equality across a gear boundary),
`ROUNDING_POLICY_UNRESOLVED` (422 — a published row resolves neither a row-level
`rounding_policy_ref` nor a tenant default; the §17.4 no-implicit-rounding rule, registered
into the pipeline by the Foundation itself), the shared money checks (§2.2) —
`PRECISION_EXCEEDED` (422 — precision above the currency's ISO 4217 minor unit),
`AMOUNT_NEGATIVE` (422 — amounts are ≥ 0), `CURRENCY_INVALID` (422 — not a valid ISO 4217
code), the aggregate validation
report envelope (422 — enumerating blocking `violations[]` plus advisory `warnings[]`), and
publish-accepted/pending (202).

**Status rendering — the 422s above are architectural, not wire (normative).** The `422`
annotations in this set say *unprocessable content*: the request was understood and the
catalog refuses to publish it. The platform's `CanonicalError` model has **no 422 category**
at all (`InvalidArgument`, `FailedPrecondition` and `OutOfRange` all render **400**), so every
architectural 422 in this design set — here and in every slice — reaches the wire as a **400
carrying its wire code**, and **the code string is the discriminator a consumer matches on,
not the status**. This is the sibling ledger's rule verbatim
(`../../ledger/docs/design/02-audit-immutability-observability.md`), and the reason it is
stated once here rather than per occurrence. Two consequences bind the implementation: a
rejection is classified by what it *is*, so a retriable conflict on mutable state stays a
**409** (`ABORTED`) rather than collapsing into the 400 bucket — the **five** conflicts above are
exactly that class (three until 2026-08-02, when `IDEMPOTENCY_KEY_IN_FLIGHT` (D-143) and
`OPEN_DRAFT_REVISION_EXISTS` (D-146) joined them; both were classified by this rule, not by the
section they were found in) — and an endpoint MUST NOT declare a 422 response in its `OpenAPI`
registration, because no path can produce one.

**Collection pagination (normative, D-125, 2026-08-01):** every collection, history and audit
read surface of this gear **paginates**: `limit` (server default 100, hard cap 1,000 — the
unit the export SLO is expressed in) plus an **opaque `cursor`**, with `next_cursor` returned
on every page until the result is exhausted. Ordering is **stable and append-consistent** —
commit/append order on history and audit reads, so a cursor walk concurrent with writes never
skips or duplicates a row at or before the cursor; a deterministic key order on catalog lists.
Offset/`$skip` pagination is not offered (unstable over append-only stores at the ≥ 7-year
retention). Slice surfaces (`/bss-pricing/v1/plans*`, `…/prices`, `/bss-pricing/v1/price-overlays`,
`/bss-pricing/v1/approvals`, `/bss-pricing/v1/history`, `/bss-pricing/v1/audit`, batch reports' row lists)
inherit this contract rather than restating it; exports stream the same order in bounded
chunks, and the p95 ≤ 5s / 100 records SLO applies **per page/chunk** — before D-125 that SLO
was expressed per page while no page contract existed anywhere in the set.

**Route shape (normative, D-140, 2026-08-02):** every REST surface of this gear is served
under the gear's service prefix — `/bss-pricing/v1/{resource}`, where `bss-pricing` is the
registered gear name — and an action on a resource is a **sub-resource segment**
(`…/{id}/publish`, `…/{id}/start`), never a colon-suffixed custom method. Slice surfaces
inherit this shape rather than restating it, as they inherit the pagination contract above.
The convention is the platform's (`/{service-name}/v{N}/{resource}`, the sibling ledger's
`/bss-ledger/v1/…`) and is enforced mechanically at build time, so it binds the documented
paths as well as the implementation.

### 3.4 Internal Dependencies

- **`toolkit-db`** — transactional persistence for the append-only history, the owned window store (Slice 7, D-03), the projected read model, the outbox, and the audit store.
- **Coordination lease library** — singleton coordination for read-model warm re-drive and the window activation/expiration job (Slice 7).

### 3.5 External Dependencies

- **Catalog registry (Product & SKU)** — published `skuId`, `bundle` SKU type, `meteringUnit` declaration, `PlanTier` taxonomy; the **sole** `CatalogVersion` incrementer. Bidirectional `CatalogVersion`-increment contract (`cpt-cf-bss-pricing-contract-registry-catalogversion`).
- ~~Effective-dating PriceWindows use case~~ — **consolidated into this gear** (Slice 7 owns the window store, state machine, activation job, and `PriceWindow*` emission — D-03); `cpt-cf-bss-pricing-contract-pricewindow` is thereby internal, not an external boundary.
- **Tariffs / Subscriptions / Rating / Billing** — consume the read model / events; their payloads are refined in the consumer-contracts and capability slices.

### 3.6 Interactions and Sequences

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-author-publish-fnd`

**Author → validate → approve → publish → CatalogVersion.** A slice authors/updates draft rows
(ETag checked, idempotency-deduped); submit runs the ValidationPipeline as a **pre-check** and
routes a material change through the Slice 5 approval gate; on approval (or immediately for a
below-threshold change) the publish commit **re-runs the aggregate rule set inside the commit
transaction** (approval approves content; the commit re-validates state — a failure at commit
voids the approval and returns the subject to draft with the report); on success the row set
transitions to `published` (append-only), the EventOutbox enqueues `PlanPublished` with a
**pending** version ref, and the SnapshotStamper requests a `CatalogVersion`. The registry batches approved publishes and emits
`CatalogVersionPublished`; the ReadModelProjector warms the projection and marks completion, or
the publish is marked degraded (`PlanPublishDegraded`). `pricingSnapshotRef` pins the committed
version. No intermediate state exposes a rateable-but-incomplete plan. A
`pricing_catalog_version_ref` still `pending` past the max batching-delay SLO raises a
Critical alarm (`pricing.catalogversion.commit_overdue`) and surfaces on the publish status
API; a `CatalogVersionPublished` batch that omits an expected pending ref is treated the same
— remediation is a registry re-request, never a silent re-emit.

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-readmodel-resolution-fnd`

**Consumer read-model resolution.** A consumer pins one **pin-eligible** `CatalogVersion` (§4.4,
D-101 + D-114: committed, every subject row of that version warm-complete, *and* every earlier
version itself pin-eligible) and resolves the
complete frozen read model via `pricingSnapshotRef` — no draft read, no default substitution,
monotonic per version; the pin never lags the newest pin-eligible version by > 5s.

### 3.7 Database Schemas and Tables

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-schema-fnd`

Foundation-owned tables (tenant-scoped, SecureORM). Money columns are integer minor units at
the currency's ISO 4217 scale. **Table-naming discipline (normative):** every physical table
of this gear carries the gear-name prefix **`pricing_`** — matching the sibling ledger's
`ledger_*` convention (`ledger_journal_entry`, `ledger_idempotency_dedup`, …). Domain entity
names (`Plan`, `Price`, `ReadModel`) stay unprefixed; the prefix applies to physical tables
only, including slice-owned tables in every other slice design.

- **`pricing_plan`** — keyed **`(plan_id, revision)`** (composite PK — the draft-revision-row model, D-56, 2026-07-28 review fix, confirmed 2026-07-31), `tenant_id`, `sku_id`, `plan_tier`, `billing_cycle`, `lifecycle_state` (`draft`|`published`|`superseded`|`retired`|`abandoned` — `superseded` added by D-90; the enumeration had omitted it in the same paragraph that describes the flip, 2026-07-31 review fix; `abandoned` added by **D-145**, 2026-08-02, the terminal state a discarded draft revision takes instead of being deleted), `available_from`/`available_to`, `created_by` (pseudonymous principal id of the authoring actor — 2026-07-28 review fix: the Slice-12 history surface reads it under `plan × read`, so actor identity never requires the Auditor-only `pricing_audit_log`), ETag/row-version. Published revision rows are immutable **in content** — their only sanctioned in-place mutations are the state-machine `lifecycle_state` flips (`published → superseded` when the next revision's publish commits — **D-90, 2026-07-31 review fix: the plan-revision analogue of the price rows' flip-at-commit**, so a partial `UNIQUE (plan_id) WHERE lifecycle_state IN ('published', 'retired')` (**widened by D-128**, 2026-08-01 review fix — the predicate previously read `= 'published'`, which held no row at all once retirement flipped the only current revision) holds at most one **current** revision and "the current revision" is unique by construction for the projector, the sellability lifecycle predicate, and every referential check — and `published → retired` on the current revision at retire, a **publish unit** in its own right (D-128, §4.2); D-56's own framing: in-place mutation survives only as state-machine flips — 2026-07-30 review fix); only the plan's single open `draft` revision row mutates content, through DraftStateMachine transitions (partial `UNIQUE (plan_id) WHERE lifecycle_state = 'draft'`; `lifecycle_state` per §3.2), fully audited — the normative revision model is §4.3. **`revision` is an identity, not a counter (D-145, 2026-08-02, found while building the draft-authoring plane):** it is minted `max(revision) + 1` over the plan's own rows and **never re-minted**, so a discarded open draft is flipped to the terminal `abandoned` state rather than deleted (its child copies dropped, the flip audited exactly as the deletion was) and the number it consumed stays consumed — deleting the row freed the number, and a re-minted `(plan_id, revision)` is a durable name pointing at two distinct rows, under which a stale ETag passes its precondition against the wrong one (§4.3 carries the argument and the rejected alternatives). `abandoned` is terminal and sits **outside both** partial `UNIQUE` predicates on this table — outside `WHERE lifecycle_state = 'draft'`, so a replacement draft opens immediately **on any plan that has published at least once** (a plan whose *only* revision is `abandoned` is the exception, and the index is not what stops it: §4.3), and outside `WHERE lifecycle_state IN ('published', 'retired')`, so "the current revision" is untouched — which is what lets the state be added without disturbing D-90's or D-128's uniqueness guarantees; **child shape tables version with the revision, copy-on-new-revision (D-83 — [`02-plan-definition.md`](./02-plan-definition.md) §6)**. **Physical enforcement extends beyond `pricing_price` (2026-07-31c review fix, L-2):** published plan-revision rows and every revision-scoped child/composition table (S2 phases/add-on rules/descriptors, S8 bundle tables, S9 overlay revisions/lines, S10 grants/composites) carry the same `REVOKE` + column-whitelist trigger discipline — permitted UPDATEs are exactly the sanctioned `lifecycle_state` flips on the revision-carrying rows; child rows are physically immutable once their revision publishes (draft-revision rows and their copies stay freely mutable/deletable). The projector's re-drive reads truth rows (§4.4), so without the physical guard an unsanctioned UPDATE would silently change a frozen version at re-warm — the same argument that put the trigger on `pricing_price`.
- **`pricing_price`** — `price_id` (PK), `tenant_id`, the **canonical scope-key columns** (`plan_id`, `currency`, `region`, `price_overlay`, `phase`, `price_eligibility`, `charge_kind`, `cohort` — `none` unless `existing_grandfathered`, ADR-0002), `amount_minor`, `model_kind`, `tax_inclusive`, `billing_timing` (recurring), evaluation-policy columns (usage), `rounding_policy_ref` (nullable named rounding-policy id — PRD §17.4: publish resolves the row-level reference, else the tenant default from `pricing_policy_object`, else fails `ROUNDING_POLICY_UNRESOLVED`; the **resolved** policy id freezes into the read model / snapshot, definition + application stay downstream in Tariffs/Billing), `tax_category_ref` (nullable — Slice 4's column and the sole per-row source of truth, D-110; **the resolved *effective* category freezes into the read model / snapshot exactly as the rounding policy does** — **D-154**, 2026-08-03: the descriptor contract owes Billing a category it can post without re-querying, and the fallback half of that value lives in the mutable per-`(tenant, region)` region taxonomy), `grandfather_until`, `supersedes_price_id`, `lifecycle_state`, `created_by` (pseudonymous authoring principal — the history-export actor field, Slice 12), **ETag/row-version** — the row's **own** optimistic-concurrency token (**D-141**, 2026-08-02, found while building the draft-authoring plane). It is never derived from the plan's: a per-row bulk conflict means nothing if every row of a plan shares one version, and three surfaces already required the token to be per price row while this section declared it on `pricing_plan` alone — S3 §5's `PATCH …/prices/{priceId}`, the bulk import's "existing **draft** rows under **their** ETags" whose conflict fails "**only that row**" ([`12-operator-efficiency.md`](./12-operator-efficiency.md) `inst-bk-phase2`, D-118), and `inst-co-single-pending`'s "**ETag protects rows**, this rule protects change units" ([`07-pricewindow-linkage.md`](./07-pricewindow-linkage.md)) — so a reader building the table from here gave `pricing_price` no version column at all and all three rules became unimplementable. **Every** mutating verb on a `draft` price row presents it: `PATCH` (unchanged) and `DELETE` (new — its S3 idempotency cell was empty, so a draft row could be destroyed under an unknown version, reopening on the one verb that leaves nothing behind to reconcile the lost update `fr-concurrent-edit` closes for `PATCH`). A mismatch is `STALE_VERSION` (§3.3); an absent precondition is a malformed request under the existing validation envelope — **no new code**. The scope is deliberately `pricing_price` draft rows: `DELETE …/price-windows/{windowId}` carries an empty cell too and is **not** moved, because window cancellation is an always-material publish unit (D-62, D-99) whose concurrency is governed by `inst-co-single-pending`. **Two partial `UNIQUE` indexes over the scope key, one per plane, with disjoint predicates.** (1) The **published**-plane index (`WHERE lifecycle_state = 'published'` — sufficient on its own under the flip-at-commit rule (§4.3: the predecessor reads `superseded` the instant its successor commits), and the only expressible form anyway — a partial-index predicate sees the row's own columns, and the supersession link lives on the **successor** row; 2026-07-30 review fix) enforces at most one current row per key — **temporal `PriceWindow` non-overlap and coverage are enforced by the publish-time validation pipeline (Slice 7, gear-owned per D-03), not by this index**, so a **superseded** predecessor (its window still active until the changeover) and its published successor with a scheduled window legally coexist. (2) The **draft**-plane index (`WHERE lifecycle_state = 'draft'`) guards the authoring plane (**D-148**, 2026-08-02, found while building it). Nothing covered that plane, while D-21 puts scope-key duplication in the **save-time** row-local set — a check decided by a read — so two concurrent creators on one key both read "absent", both insert, and the duplicate goes undiscovered until one of them publishes, at which point the operator is told, correctly and uselessly, that a row they authored days ago collides with one they cannot see. That is the opposite of what D-21's save-time placement is for. Under the index, D-21's read stays the fast, explanatory path and the index becomes the guarantee — the read-then-index arrangement the published plane already has — and a violation renders as the existing `DUPLICATE_SCOPE_KEY` (§3.3): **no new code**. The two predicates are disjoint by construction, so a draft successor legitimately coexists with the published row it will supersede, and the only-expressible-form argument above is untouched — it was an argument about which rows the *published* index can see, never an argument against a second predicate. **Nor can a staged composition put a *second* draft on the key** (verified 2026-08-02 against the composition paths, recorded in D-148's body): the supersession unit composes only on a key whose predecessor window it can shorten ([`07-pricewindow-linkage.md`](./07-pricewindow-linkage.md) `inst-su-compose`, which fails compose on a dormant key), so the key already carries a `published` row — which is exactly what makes a hand-authored draft on it impossible, refused at save as a duplicate active scope key ([`03-price-structure.md`](./03-price-structure.md) `inst-pr-return`, D-21) and refused per-row on the bulk plane (`IMPORT_TARGETS_PUBLISHED`, D-118); a second *unit* on a held key is refused by `inst-co-single-pending`, and both bulk mechanisms fail per-row against a pending one (`inst-bk-phase1`, `inst-mp-pending`, D-35). The cutover stages nothing here at all — it is "not a table" but an approval-unit payload, and its successor is **inserted** on the published plane inside the commit (D-100), its grandfathered copy landing on a **new generation key** (`inst-co-copy`). So this index refuses exactly what D-21's save-time read cannot catch and nothing else: two concurrent creators on a key no published row occupies. Rejected: widening the published index to `IN ('draft', 'published')`, which would make a draft successor collide with the very row it supersedes. Consequence for bulk import (D-118 — draft plane, per-row commits): a batch carrying two rows on one key now fails the second **on the index** rather than admitting both, and `inst-bk-phase1`'s per-row report names that case. Append-only via `REVOKE UPDATE, DELETE` + `BEFORE UPDATE/DELETE` trigger with a **column whitelist**: the trigger rejects any UPDATE of a published row except (a) `lifecycle_state` transitions permitted by the state machine (`published → superseded` on supersession/cutover) and (b) monotonic tightening of `grandfather_until` (setting it when null, or moving it earlier); all price/scope/model columns **and the row-version column** are immutable and DELETE is always rejected — controlled transitions run through the engine's transition path, never ad-hoc SQL. **The draft plane is guarded for transitions too (D-153, 2026-08-03).** A column whitelist is scoped to *published* rows by construction, so it says nothing about where a **draft** row may go, and the price row's state machine ([`03-price-structure.md`](./03-price-structure.md) §4) has exactly one edge out of `draft` — to `published`. A draft row moved straight to `superseded` would satisfy every constraint on this table and land **outside both** partial `UNIQUE` predicates above: its key would read free on the published plane *and* on the draft plane, so the guarantee D-148 had just bought — the second concurrent creator is refused — is undone by one UPDATE, and `inst-ps-nodelete` then makes the ghost undeletable, on a key no supersession chain reaches because the row was never current. The trigger therefore constrains the draft row's `lifecycle_state` as well: `draft → draft | published` and nothing else, exactly as `pricing_plan`'s does for its own state set (`draft → draft | published | abandoned`, D-145). **No new code** — no API offers the transition and no caller can provoke it; this is the physical floor under a state machine the engine already honours, the same posture as the D-148 index. **The row version freezes with the published row's content (D-141, 2026-08-02).** It joins the frozen whitelist beside the price/scope/model columns, and neither sanctioned in-place mutation moves it: not the `lifecycle_state` flips, not the monotonic `grandfather_until` tightening. An entity tag that moved under a representation the engine forbids changing would tell a client its cached copy is stale when it is not, and the tag exists for the **draft** plane, where content really does change and D-141's precondition rule binds; on the published plane there is no caller-driven mutation for a precondition to guard, which is why that rule is scoped to draft rows.
- **Price history** — history is the set of superseded rows retained **in `pricing_price` itself**, keyed by `supersedes_price_id`; no rows are ever moved or deleted (no separate history table).
- **`pricing_read_model`** — the projected frozen view keyed by **`(tenant_id, catalog_version, subject_kind, subject_ref)`** with a per-row `warm_completed` marker; monotonic per `catalog_version`. **Storage is a per-subject delta (D-86, 2026-07-30; subject-typed by D-91, 2026-07-31)**: `subject_kind ∈ {plan, price_overlay, overlay_index, group_membership}` (extensible), and a version's rows are exactly the subjects of the publish units that produced it — never a full tenant copy (≤ 5s interactive coalescing would explode one). A plan publish projects its plan-subject row (`subject_ref = plan_id`, exactly the D-86 semantics); a **`PriceOverlay` publish unit projects one overlay-subject row** (the overlay document: lines, amounts, dating, disclosure, lifecycle) and **never re-projects targeted plans** — Tariffs joins overlays to base rows at evaluation per the §9.2 contract, so a `global`-scope overlay commit writes one row, not a tenant's worth; a **membership publish unit projects one membership-subject row** per payer record (D-06's units thereby have a defined read-model representation). An overlay publish unit additionally re-projects an **`overlay_index`** subject — the live overlay id set with each overlay's interval and precedence (**D-112**, 2026-07-31 review fix): per-subject resolution answers "overlay X at pin V" but evaluation needs the *set*, and without an index the only path was a `DISTINCT subject_ref` scan over ≥ 7 years of overlay deltas on the order-time p95 < 100ms path ([`09-price-overlays.md`](./09-price-overlays.md) §7). **The index is sharded and horizon-bounded (D-133, 2026-08-01 review fix):** `subject_ref = (scope_class, scope_value)` (a `global` sentinel value for the classless one), and a shard carries only overlays whose own interval intersects `[projection_time − H, ∞)` on the D-121 horizon. D-112's accounting — "two delta rows per commit, still O(publish units)" — counted **rows and not bytes**: as a single tenant-wide document the index was O(live overlay count) per row, rewritten whole on every commit and retained on the ≥ 7-year truth horizon, i.e. O(commits × overlays) of storage and O(overlays) of write amplification per commit, on the object the order-time read path touches. Sharded, a commit rewrites exactly one shard (two, when a revision moves the overlay's scope value), and an evaluation reads the ≤ 6 shards its payer context can match as point lookups. Resolving `(pin V, subject S)` reads S's row with the greatest `catalog_version ≤ V` whose `warm_completed` is set (one indexed read on `(tenant_id, subject_kind, subject_ref, catalog_version DESC)`, inside the p95 < 100ms budget). **Retention**: delta rows are retained on the same horizon as the append-only truth history (≥ 7y, jurisdiction-configurable, audit-aligned) — growth is O(publish units), the truth tables' own order; compacting superseded deltas beyond the horizon is an ops knob, never a semantics change. **Per-delta size** is bounded by the D-121 projected-set rule (§4.4): a plan delta carries the rows/windows intersecting the `H` horizon, never the plan's whole accumulated history.
- **`pricing_catalog_version_ref`** — `pending` vs `committed` version linkage per publish.
- **`pricing_pin_frontier`** — PK `tenant_id`; `catalog_version` + `advanced_at`. The materialized **pin-eligibility frontier** (D-136, §4.4): advanced only forward, and only by the ReadModelProjector inside the transaction that completes the frontier's next version in order. It is what `GET /bss-pricing/v1/catalog-version/frontier` serves, what the ≤ 5s pin-lag rule is measured against, and what `pricing.readmodel.pin_eligibility_overdue` reads — the D-101/D-114 predicate is otherwise a recursive scan of the delta store on the p95 < 100 ms path, with no owner and no surface.
- **`pricing_policy_object`** — the approval-threshold and tax-display policies (fail-safe defaults), the tenant **default rounding policy** (a named rounding-policy id; optional — a tenant without one simply requires every published row to carry its own `rounding_policy_ref`, per the §17.4 fail-closed rule), the **enforced-migration notice period** (days; default floor 60 — D-49, validated by Slice 11 at scheduling), and **every per-tenant configurable this gear promises** (**D-152**, 2026-08-03, found while building the Slice-2 validators): the descriptor required-set extension (additional required descriptor keys, matched against `pricing_plan_descriptor_set.additional_fields` — S2 `inst-ds-sufficient`, P5) and the §14 soft caps and interval caps (tier bands per row, price rows per plan, `customEveryNDays`/`customEveryNMonths`). Those numbers and that extension were each described as tenant-configurable in a ratified NFR or a pinned assumption while **no document named where they are declared**, so a promise of per-tenant configuration had no carrier and the gear's own configuration section is per **deployment**; this bullet is the carrier, and a tenant with no entry takes the ratified launch default. Nothing here is on a resolution path: these are authoring-time policy reads, like the two that were already here.
- **`pricing_operator_flag`** — operator-plane drift/divergence flags, keyed `(tenant_id, subject_ref, flag)`; set/cleared by the external-signal handlers (audited): `tier_divergent` (Slice 2), `grants_divergent` (Slice 6), the tax-readiness divergence (Slice 4), `meter_binding_divergent` (Slice 2 — a registry metering-unit binding/dimension-set change diverging from a published plan's frozen mapping; 2026-07-31 review fix). **Never part of `pricing_read_model`** (D-85): a drift flag has no publish unit — consumers keep resolving the frozen values; operators read the flags via the authoring surfaces (`plan × read`) and the existing alarms.
- **`pricing_idempotency_dedup`** — PK `(tenant_id, operation, client_key)`, a request-payload digest, and the two response columns (`response_status`, `response_body`); the at-most-once gate + replay-response source; the idempotency check precedes the ETag check. **The row has exactly two states, and the TTL is evaluated at claim time (normative, D-142, 2026-08-02, found while building the draft-authoring plane).** The shape this bullet described — a hash beside a stored response — could not represent the instant the gate actually needs: the at-most-once gate **is** the primary-key INSERT, so the row must exist *before* the guarded operation has produced any response, and the only way to force the old shape was to seed a fabricated status into a column whose meaning is "this is what the caller was told" when nobody had been told anything. Five clauses. **(1)** The row is `claimed` (both response columns null) or `answered` (both set) and nothing else, with `(response_status IS NULL) = (response_body IS NULL)` enforced **physically** on both backends; the claim INSERT is the gate and precedes the guarded operation, and no synthetic response is ever seeded. **(2)** Expiry is evaluated **at claim time**, against the row as read: there is no reaper, and nothing in this gear deletes a dedup row — the store's **retention** is deliberately not decided here and stands as an open fork in the register; answering it cannot disturb this clause either way, since a row that has vanished is indistinguishable from a key never claimed and the INSERT path simply wins. **(3)** Expiry is evaluated **before** the payload-digest comparison; the reverse order hands the first payload to touch a key ownership of it forever, which makes the ratified 24h TTL (§1.2) unreachable and therefore not a bound at all. **(4)** A takeover of an expired row is a **compare-and-swap on the row as read**, so two racing takeovers cannot both win; the loser claimed nothing and executes nothing. **(5)** `record_response` is **write-once**: a second answer against an `answered` claim is neither an error nor an overwrite but the **replay path**, returning the **stored** response — exactly what `fr-mutation-idempotency` promises a retry, and what keeps an ordinary retry of a request that both exists and succeeded from reaching the caller as a not-found refusal. Rejected: a **reaper** as the expiry mechanism, which turns expiry into a background-timing property that must then race the compare-and-swap; and **seeding a synthetic `202`** at claim, under which the replay path would serve a response the gear invented. **A duplicate therefore has three outcomes, not two (D-143, 2026-08-02, flagged for veto).** A replay with a matching digest returns the stored response; a mismatching digest is rejected with `IDEMPOTENCY_PAYLOAD_MISMATCH` (never replayed, never re-executed); and a duplicate arriving against a `claimed`, unanswered key is refused with `IDEMPOTENCY_KEY_IN_FLIGHT` (409, §3.3). The third outcome is reachable **with no contract violation by anyone** — it is clause (4)'s losing caller, holding a request against a key that is claimed and unanswered, whose payload may differ from the winner's so that neither of the first two promises holds either — which is why stating the one-transaction contract normatively cannot close it, and why the surface layer needed something to say. At-most-once is never violated on that path: the loser executes nothing.
- **`pricing_outbox`** — the transactional event outbox (frozen event names, dedup/correlation keys, `(tenantId, aggregateId)` ordering).
- **`pricing_audit_log`** — append-only actor/before-after/approval trail, hash-chained per D-14 and **segmented per `(tenant_id, chain_id)`** with a periodic per-tenant roll-up chaining the segment heads (**D-135**, 2026-08-01 review fix — `chain_id` = the audited subject's aggregate: plan, overlay, payer, policy, bulk operation; a single per-tenant chain serialized *every* mutation of a tenant behind one head, inside the mutation transaction, which the ≥ 50 rows/s repricing SLO never accounted for); normative: [`05-governance.md`](./05-governance.md); ≥ 7-year configurable retention.

**Governed backdated reference rows are NOT in `pricing_price` (D-76).** The historical-import
path writes a disjoint Slice-5-owned store, `pricing_historical_price`
([`05-governance.md`](./05-governance.md) `inst-bd-store`), which is never window-linked, never
projected into `pricing_read_model`, and never an input to coverage, sellability or
`CatalogVersion` addressability; its only reader is snapshot synthesis
([`11-lifecycle.md`](./11-lifecycle.md) `inst-sy-select`). Every statement in this design about
published `pricing_price` rows — §4.3 immutability, the scope-key partial `UNIQUE`, the
REVOKE/trigger discipline, "consumers resolve only committed versions" — therefore holds
**without an exception class**, and a reference row cannot reach live resolution through a
forgotten query predicate.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-pricing-deployment-fnd`

Stateless authoring/publish + read-model service over a shared `toolkit-db` backend;
background work (read-model warm re-drive) is coordinated as a singleton via the coordination
lease library. The read path is served from the projected read model and fails closed on
outage. Deployment specifics are platform-standard for a BSS gear, except the residency
constraint (`cpt-cf-bss-pricing-nfr-data-residency`): a residency-bound tenant's gear-owned
stores — `pricing_*` tables (incl. `pricing_audit_log`), read-model projection, outbox,
backups/DR replicas — are pinned to an in-jurisdiction deployment cell; the gear itself never
replicates across cells, and a residency-bound tenant configured onto a non-compliant cell
fails deployment config validation (fail-closed).

## 4. Additional Context

### 4.1 Canonical Scope Key (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-scope-key`

The single scope key for **row-uniqueness, supersession, `PriceWindow` non-overlap, and window
coverage** is:

```text
(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)
```

- Axis defaults: `priceOverlay = base` (rows authored here always carry `base`; partner/orgTier/brand overlays are separate `PriceOverlay` rows evaluated downstream by Tariffs), `phase =` **the plan's terminal `phase_id`** (D-19: the axis is always uuid-typed — for a phased plan its authored terminal phase; for non-phased/one-time plans an **implicit terminal phase row** (kind `evergreen`) is auto-created at plan creation and its id is the default; the literal `evergreen` survives only as the phase *kind*), `priceEligibility = all_subscriptions`, `chargeKind` per row, `cohort = none`.
- `cohort` (ADR-0002) is the **grandfathering generation discriminator** — the UTC cutover instant that created the generation, **at the millisecond quantum §2.2 fixes** (D-144, 2026-08-02): this axis is matched for *equality*, so its resolution is part of the key's meaning and not a rendering detail. Publish validation enforces `cohort ≠ none ⇔ priceEligibility = existing_grandfathered` (`COHORT_ELIGIBILITY_MISMATCH` — the rule was normative from the start and had no code of its own; named here 2026-08-02 while implementing the axis, since every other publish-blocking rule in this set carries one and a rule with no code cannot be reported); every cutover creates a **new** generation on its own key, so repeated repricing with per-cohort retention never violates non-overlap. Within the `existing_grandfathered` class, Tariffs selects the row whose `cohort` equals the cohort of the subscription's **pinned price id** (`pricingSnapshotRef`) — and, when that pin carries `cohort = none` (the **bootstrap** case, D-126: the subscription predates the key's first cutover, so its pin is on a non-grandfathered row), the generation whose `cohort` equals the **pinned row's window `effectiveTo`**, which is by construction the instant of the cutover that closed it — **the equality is at the millisecond quantum on both sides** (D-144), which this comparison in particular depends on, since the two instants are produced by different code paths in different gears and two instants denoting the same moment at different resolutions are not equal; if no generation carries that instant the class contributes **no** candidate and resolution continues down the class order (the pinned row was closed by a supersession, not a cutover — that subscriber is not grandfathered). Class ordering (most-specific-wins) is unchanged. Unrelated to `customerGroup` segment pricing.
- `chargeKind ∈ {recurring, usage, one_time, one_time_setup}` distinguishes the components a single plan legitimately carries at once: a hybrid plan holds a `recurring` **and** a `usage` row (optionally a `one_time_setup` row) on one `planId`, and a one-time plan's base row is `one_time` — so they are **distinct keys**, not duplicates.
- `brand` is **NOT** a price-row axis: brand-differentiated pricing is a **brand-scoped `PriceOverlay`** overlay (manifest §4.1 invariant).
- This key **extends the manifest `(plan, currency, region, priceOverlay)` key additively** with `phase`, `priceEligibility`, `chargeKind` (ADR `cpt-cf-bss-pricing-adr-canonical-scope-key`) and `cohort` (ADR `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`), and **supersedes** the narrower effective-dating `(plan, currency, region, priceOverlay)` key for normative purposes.

Because `priceEligibility` and `cohort` are part of the key, a grandfathered generation and
its successor — and any number of prior generations — are **distinct keys** that hold active
windows concurrently at the same instant without violating non-overlap (§4.3).

**`grandfatherUntil` is a grandfathered-row field (normative, D-147, 2026-08-02, found while
building the draft-authoring plane).** `grandfather_until` is non-null **only** on a row whose
`priceEligibility = existing_grandfathered`; a value anywhere else fails publish
(`GRANDFATHER_UNTIL_FORBIDDEN`, 422 — declared by the slice that owns the eligibility machinery,
[`07-pricewindow-linkage.md`](./07-pricewindow-linkage.md) §5). It is the `cohort` rule's sibling
and takes the same shape — one axis-conditioned field, one code — and it is stated here because
until now the pairing was enforced by a column check and by nothing else: §4.3's
only-permitted-mutation clause, `inst-gs-bound`/`inst-gs-tighten` and `inst-cl-resets` (which
resets the two together) all read as if the rule held, while `inst-el-fields` publishes
`grandfatherUntil` "per row" unqualified and the PRD glossary lists it among a **Price row**'s
optional fields. A constraint with no rule gets no code, so its violation reached the caller as an
internal fault — a 500 for a request the operator could have reshaped. Rejected: reading
`grandfatherUntil` as a **general per-row availability bound**, which the unqualified wording
admits. The set already carries two mechanisms for "this row stops being sellable at `T`" — the
window's `effectiveTo` and the plan's `available_to` — and a third that only the eligibility
machinery derives a signal from (`inst-gs-expire`'s read-derived `EligibilityExpirySignal`, whose
entire meaning is "re-bind at the next renewal") would give one fact three unreconciled homes.
The converse is deliberately **not** stated: a grandfathered row with a null `grandfatherUntil`
is indefinite.

### 4.2 Publish-Through-The-Engine Contract (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-publish-contract`

Every state change that reaches production follows one path:

1. **Author draft** — create/update/clone in `draft` (ETag-checked, idempotency-deduped). Only `draft` rows are mutable; a never-published `draft` **price** row is deletable, while a plan's open `draft` **revision** row is abandoned rather than deleted (D-145, §4.3).
2. **Validate (fail closed)** — the aggregate validation pipeline runs the §17.4 rule set plus every slice-registered rule; a single failure blocks the submission with an enumerated report. This step is a **pre-check**: the same rule set re-runs inside the publish-commit transaction of step 4 (approval approves content; the commit re-validates state — a commit-time failure voids the approval and returns the subject to draft with the report). No `PlanPublished`, no read-model warm on any failure.
3. **Approve** — a material change (above the configured threshold, or a first publish) requires the submitter **plus ≥ 1 independent approver** (two distinct principals; self-approval rejected + audited). Fail-safe: the two-person rule applies unless an explicit threshold is configured and the change is below it and it is not a first publish. Pending approval is **not** a `lifecycle_state`: the subject stays `draft` and remains mutable — the open Slice 5 approval record marks it, and any mutation of the subject **voids** that record ("returns to draft" in the PRD means the approval record closes).
4. **Freeze + emit** — the publish commit re-runs the pipeline, transitions the row set, and stamps the catalog-side `pricingSnapshotRef` identifiers; the frozen event set is enqueued transactionally (`PlanPublished` with a **pending** version ref).
5. **Version + warm** — the registry (sole incrementer) batches approved publishes and emits `CatalogVersionPublished`; the read model warms to the 5s SLO or the publish is marked degraded (`PlanPublishDegraded`). No intermediate state exposes a rateable-but-incomplete plan.

Publish units are not only plans: Slice 9's `PriceOverlays` and customer-group
membership mutations publish through the **same engine** (validation → pending ref → warm;
D-06) — nothing becomes consumer-visible outside a committed `CatalogVersion`.

**Plan retirement is a publish unit too (normative, D-128, 2026-08-01 review fix).** The
`published → retired` flip runs the same engine path (validation → pending `CatalogVersion`
ref → warm) and **re-projects the plan subject**, because the plan's `lifecycle_state` is
sellability predicate (4) ([`07-pricewindow-linkage.md`](./07-pricewindow-linkage.md)
`inst-sg-surface`) and that predicate is required to be evaluable from the *pinned* read model
(`inst-sg-pinned`). Before this rule retirement requested nothing and warmed nothing —
[`11-lifecycle.md`](./11-lifecycle.md) `inst-rt-event` discharged it as "the read model flags
the plan not-sellable", an in-place mutation of a frozen version that D-85/D-99 forbid, and
[`../PRD.md`](../PRD.md) §17.5's increment table listed no retirement class at all. It cannot
self-heal like other lagging facts: a retired plan can never publish again (no
`retired → draft`/`published` edge, and its open draft revision is **abandoned** in the same
transaction — **D-145**, 2026-08-02: the revision row is flipped to the terminal `abandoned`
state rather than deleted, so the number it consumed is never re-minted (§4.3), and the flip is
audited exactly as the deletion was; D-128's own three clauses are unchanged), so **no later
publish would ever re-project it** and the read model would
advertise a retired plan as sellable permanently. The projection source is correspondingly the
plan's **current** revision — `published` *or* `retired` (§4.4/§3.7) — so a retired plan keeps
a resolvable delta for the in-flight subscribers D-51 preserves coverage for.

**Window mutations are publish units too (normative, D-99, 2026-07-31 review fix).** Every
committed `WindowScheduler` mutation — schedule, future-`effectiveTo` adjustment, cancellation
([`07-pricewindow-linkage.md`](./07-pricewindow-linkage.md) §5) — runs the same engine path
(validation → pending `CatalogVersion` ref → warm) and **re-projects the affected rows' plan
subject**, because window facts are what the sellability gate's predicate (1) and the D-80
coverage horizon are evaluated from and those are required to be resolvable from the *pinned*
read model (`inst-sg-pinned`). Windows are plan facts, so they need no subject kind of their
own. Before this rule the window surface requested nothing and warmed nothing, while
[`../PRD.md`](../PRD.md) §17.5's increment table already required a window edit to become
addressable in a `CatalogVersion`: a cancellation left the last-warmed delta advertising
coverage the truth side had removed (selling into the trailing void D-62/D-80/D-94 close), and
a coverage extension could not lift a horizon block until some unrelated publish re-projected
the plan. The cutover and supersession units already requested addressability
(`inst-gc-commit`, `inst-su-commit`); this closes the standalone surface. Activation and
expiry, by contrast, are **not** publish units and need none — see §4.4: the read model carries
window **intervals**, and "active at `t`" is derived at read time.

Consumers never read draft state and never substitute a default for an absent
evaluation-policy field (absence must have failed step 2). The catalog computes **no** monetary
charge, evaluates **no** overlay, and performs **no** FX.

### 4.3 Immutability and Change Mechanisms (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-immutability`

Published `pricing_price` rows are **append-only history**. A transition this section does not
sanction is refused with `LIFECYCLE_FORBIDDEN` (§3.3) — mutating a published row's content,
superseding a grandfathered row — and the refusal is enforced
twice, by the validation pipeline and by the physical guard below, because the engine is not
the only thing that can reach the table. `REVOKE` + a trigger with a
**column whitelist** reject any UPDATE except the state-machine `lifecycle_state` transitions
(`published → superseded`) and monotonic `grandfather_until` tightening — price/scope/model
columns **and the row-version column** are immutable and DELETE is always rejected (§3.7, which
states why the tag freezes with the content it names — D-141); only never-published `draft`
price rows are deletable (a plan's open draft **revision** row is abandoned instead, per the
revision rule below); there is no deletion event to fan out. Change over time uses four **distinct,
composable** mechanisms ([`../PRD.md`](../PRD.md) §17.5):

- **Versioning** — captures a structural/price change as a new immutable revision; prior rows retained as history (`PlanUpdated` / `PriceCreated`).
- **Supersession** — versioning scoped to **one canonical scope key**: a new immutable row plus opening/closing the corresponding `PriceWindow` (never overlap), within one `priceEligibility` class and one `chargeKind` — and, on usage rows, with the unit/counter-determining fields, **`model_kind`**, **`package_size`** and (on a `carry`-allowance row) **`included_allowance`** preserved (meter, granularities, aggregation windows; `SUPERSESSION_UNIT_MISMATCH` otherwise — D-82 + D-98 + D-122 + D-129, Slice 3 rule) (`PriceUpdated`). **The guard binds the *key*, not the mechanism (D-127, 2026-08-01 review fix):** it applies to **every** row that lands on an occupied published canonical scope key — the supersession unit **and the cutover's `all_subscriptions` successor**, which lands on the predecessor's own key (D-100) and inherits the same continued counter. Both sanctioned producers set `supersedes_price_id` on the successor, which is what gives the guard its comparison referent. It executes as the **supersession unit** (D-88, Slice 7): successor row + predecessor-window shorten + successor-window schedule composed gap-free, one approval unit, one local ACID commit — the only way to supersede *interactively*; the grandfathering cutover is the **second** sanctioned producer of the same flip (D-100, below). The predecessor's `lifecycle_state` flips `published → superseded` at the supersession **commit**, not at window activation — effectiveness over time lives solely in the `PriceWindow`, so the **published-plane** scope-key partial `UNIQUE` (§3.7 — the draft-plane index D-148 adds is a disjoint second one) always sees exactly one current row per key (2026-07-28 review fix, confirmed 2026-07-31).
- **`PriceWindow`** — schedules **when** a versioned/superseded row is effective (window store, state machine, and activation job owned by Slice 7 in this gear — D-03).
- **Grandfathering cutover** — one atomic approval unit that shortens the current `all_subscriptions` window `effectiveTo` to the cutover and schedules (a) an immutable `existing_grandfathered` copy and (b) the `all_subscriptions` successor, so **no coverage gap opens**. Because the successor lands on the **same** canonical scope key as the predecessor (only the copy moves to a new `cohort` key), the cutover commit **also flips the predecessor `published → superseded`** — the second sanctioned producer of that transition beside the supersession unit (**D-100**, 2026-07-31 review fix: `algo-cutover` had enumerated its whole transaction without the flip while the price-row state machine called the supersession unit the *only* path, so the commit inserted a second published row on an occupied key and died on the published-plane scope-key partial `UNIQUE`). The successor therefore carries `supersedes_price_id` and passes the **same unit guard** as an interactive supersession (**D-127**, 2026-08-01 review fix: the guard was written as "a *usage-row supersession*" and invoked only from `inst-su-compose`, so a cutover successor could flip `per_hour → per_day`, `graduated → volume` or `package_size` on a key whose counter continues — the ×24 class through its fifth door, on the one path that is always material and therefore felt safe).

**Plan revisions (D-56, 2026-07-28 review fix, confirmed 2026-07-31; child tables per D-83):**
`pricing_plan` is keyed
`(plan_id, revision)`. A published revision row is **immutable in content** (its only in-place
mutations are the sanctioned `lifecycle_state` flips: `published → superseded` when its successor
revision's publish commits — D-90, so at most one revision per plan is ever **current** (partial
`UNIQUE … WHERE lifecycle_state IN ('published', 'retired')`, widened by D-128) and "the current
revision" is storage-defined, never a max-scan convention — and
`published → retired` at retire, itself a publish unit per §4.2); a shape change on a published
plan opens a **new** revision row in `draft` (at most one open draft per plan — partial
`UNIQUE (plan_id) WHERE lifecycle_state = 'draft'`) that publishes through the standard §4.2
path and becomes the current revision, flipping its predecessor `superseded` in the same commit. The plan's identity, the scope-key axes, and the
`pricing_price` attachment stay on `plan_id` (unchanged), and the child shape tables — phases,
add-on rules, descriptor set — **version with the revision, copy-on-new-revision** (D-83,
2026-07-30 review fix): the new revision copies them with stable `phase_id`s (so the `phase`
scope-key axis and same-key supersession are untouched), the open draft edits **its own
copies**, and a published revision's child rows are immutable with it
([`02-plan-definition.md`](./02-plan-definition.md) §6) —
so "published plans never return to draft" means the published revision row and its child rows
never mutate; change is always a new revision row.

**`revision` is an identity, never re-used (normative, D-145, 2026-08-02, found while building
the draft-authoring plane).** A number minted for a plan is never minted again. An open draft
revision that is discarded — by its author, or by the retirement transaction that closes it
(§4.2; [`11-lifecycle.md`](./11-lifecycle.md) `inst-rt-cancel`,
[`02-plan-definition.md`](./02-plan-definition.md) `inst-pl-abandon`) — is **not deleted** but
flipped to the terminal `abandoned` `lifecycle_state`: its child copies are dropped, the flip is
audited exactly as the deletion was, and the row survives as a tombstone, so minting is
`max(revision) + 1` over the plan's own rows and consults nothing else. Deletion freed the
number, and `(plan_id, revision)` is a **durable name** this set already dereferences —
`pricing_plan_grant` is keyed `(grant_id, plan_revision)` (D-52/D-106), every revision-scoped
child table copies on new revision (D-83/D-92/D-106), and the audit trail records the revision it
mutated — so `plan/2` could name two distinct rows over a plan's lifetime. Nothing observes that
while the current revision never regresses, but the plan row's ETag does: a client holding the
discarded `plan/2`'s row version `PATCH`es the **new** `plan/2` believing it is editing the old
one and the precondition **passes**, most obviously at the initial version every freshly minted
revision carries. That is the lost update optimistic concurrency exists to refuse, arriving
through the key instead of the version. `abandoned` is terminal and sits outside **both** partial
`UNIQUE` predicates (§3.7) — outside `= 'draft'`, so a replacement draft opens immediately **on a
plan that has published at least once** (the never-published plan is the exception, and the
amendment below states it), and
outside `IN ('published', 'retired')`, so "the current revision" (D-90, widened by D-128) is
untouched. Consequence, stated plainly: a plan's revision numbers may have **gaps** (rev 1
published, rev 2 abandoned, rev 3 published), which the Slice-12 history surface shows an
operator. Rejected: (a) keeping deletion and forbidding `(plan_id, revision)` as a reference —
refuted by the set itself, since `pricing_plan_grant`'s primary key *is* that reference; (b)
keeping deletion and sourcing the next number from the audit log, which puts an append-only
forensic store on the authoring path. The scope is the **plan revision row**: price-row
deletability is untouched, so the price-row clause above and
[`03-price-structure.md`](./03-price-structure.md) `inst-ps-nodelete` keep their meaning.

**A plan that never published spends its id when its one draft is abandoned (normative, D-145 as
amended 2026-08-02).** "A replacement draft opens immediately" is a statement about the partial
`UNIQUE` index, and the index is not the only thing a new revision has to get past. Minting has
**two** entry points and only one of them is `max(revision) + 1`: a plan's **first** draft is
minted at revision `0` outright — there is no row to take a maximum over — while opening a
**successor** presupposes a current revision to succeed from. So the one plan the rule above does
not cover is the plan created and abandoned before its first publish: it holds exactly one row,
revision `0`, `abandoned`; it has no current revision and no open draft; and it can acquire
neither, because creating collides on the `(plan_id, revision)` primary key and opening a
successor finds nothing to succeed. The `revision` identity rule is what makes that true — before
it, deletion freed revision `0` and a retry worked — and it is **kept**, because the two ways out
are both worse. Minting `max(revision) + 1` on the create path as well would close the hole and
make a retried create naming the same id open a **second revision of an existing plan** instead of
refusing it, and refusing is what creation idempotency rests on (§3.7 `pricing_idempotency_dedup`,
[`../PRD.md`](../PRD.md) `fr-mutation-idempotency`). Exempting revision `0` from the identity rule
would let `plan/0` name two rows over a plan's lifetime — the unstable reference this whole rule
removes, reintroduced on the one number every plan starts at. A plan id is minted server-side, so
a spent id costs an operator nothing; a silent second revision costs a great deal.

**What is owed is the answer, not the state.** An authoring call naming such a plan is refused
with `PLAN_ABANDONED_NO_SUCCESSOR` (422, §3.3) — a **precondition refusal that names the state**,
where the storage layer alone produces an internal fault on one arm (the primary-key collision)
and a not-found on the other (no current revision to succeed), neither of which tells a caller
that the id is permanently unusable and that retrying is pointless. It is a code of its own rather
than `PLAN_RETIRED_NO_SUCCESSOR`, which would assert a retirement that never happened over a plan
that still has a current revision, a warm delta and a clone route forward, and rather than
`LIFECYCLE_FORBIDDEN`, which D-146 leaves holding exactly the refusals with **no alternative
action to describe** — this one has one, and it is specific: the id is spent, mint a new plan.
**This gear has no authoring REST surface yet**, so nothing raises the code today and the
repositories underneath are correct as written; the refusal belongs to the group that builds that
surface (**G7, authoring REST**, on `bss/pricing-impl`), which owes the 500 and the 404 their
replacement.

**Two refusals were narrowed out of `LIFECYCLE_FORBIDDEN` (normative, D-146, 2026-08-02, found
while building the draft-authoring plane).** Opening a revision on, or re-publishing, a
**retired** plan is `PLAN_RETIRED_NO_SUCCESSOR` (422); a second draft revision on a plan that
already holds one is `OPEN_DRAFT_REVISION_EXISTS` (409, naming the open revision). Both are
**narrowings**, not codes added beside `LIFECYCLE_FORBIDDEN`, so no refusal ends up with two
codes. A consumer must be able to tell them apart without parsing prose, because the operator's
next action differs: a retired plan can never publish again, so any successor is unpublishable by
construction and there is nothing to do, while "you already have an open draft at revision N"
names a **different and available** action — go and edit it. The second is not a state-machine
transition at all but a uniqueness conflict on the `(plan_id) WHERE lifecycle_state = 'draft'`
partial index (§3.7), so even the 422 category was a compromise; 409 puts it in the
`DUPLICATE_SCOPE_KEY` class that §3.3's status rule keeps out of the 400 bucket. What stays under
`LIFECYCLE_FORBIDDEN` is exactly the refusals with no alternative action to describe — mutating a
published row's content, superseding a grandfathered row. Rejected: giving **every** internal
refusal its own code. The read model's forward-only pin frontier (D-136, §4.4) draws the line: it
is advanced only by the ReadModelProjector and is unreachable by a caller, so a regression there
is an internal fault rather than a lifecycle refusal, and a wire code for it would document an
API a client cannot provoke. Both codes are Foundation-owned; slices reference and never redefine
them (§3.3).

An `existing_grandfathered` row is **immutable in price** and MUST NOT be superseded; the only
permitted mutation is **setting or tightening `grandfatherUntil`** (never loosening, never the
price), which is a material change. Because it is a distinct scope key (via `priceEligibility`),
it holds an active window concurrently with its successor and is **live-resolved** by Tariffs
against an immutable row — reconciling live resolution with the frozen-snapshot doctrine.

### 4.4 Read Model and pricingSnapshotRef (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-read-model`

The published read model is **monotonic per `CatalogVersion`**: a subject's row at a version is
ignored until `CatalogVersionPublished` **and** that row's `warm_completed` marker are both
present — the marker is stored **per subject row** (D-86/D-91).

**Pin-eligibility is version-level and prefix-closed (normative, D-101 + D-114, 2026-07-31
review fixes; amends D-91's resolution rule, not its storage).** A `CatalogVersion` becomes
**pin-eligible** only once `CatalogVersionPublished` has been emitted, **every subject row that
version projects carries its `warm_completed` marker**, **and every earlier version is itself
pin-eligible** (D-114) — pin-eligibility is a **monotonic frontier**, and "the newest completed
version" — the referent of the ≤ 5s pin-lag rule below — means that frontier's edge. Resolution
stays per subject (greatest completed version ≤ the pin), and under the two rules together the
per-subject fallback is **never observable at a pinned version**: every subject the pin's
version touched is warm at it, and every older delta a fallback can reach lies at or below the
frontier, so it is complete and frozen. Each condition closes a real divergence. Without
version-level eligibility (D-101) a *single* pin resolved two different contents over time —
plan `P` at `V-3` before its delta warmed and at `V` after. Without the prefix (D-114) the same
divergence survived one version out: with `V5` degraded (plan `A`'s warm outstanding — a
re-drive continues past the SLO with no bound, §1.2/§3.6) and `V6` fully warm, `V6` was
pin-eligible while a pin of it resolved `A@V4` — and resolved `A@V5` once the late warm landed,
because the fallback reaches **through** the pin into older deltas that nothing else required to
be complete. A version that has not become pin-eligible within the max batching-delay SLO
raises Critical (`pricing.readmodel.pin_eligibility_overdue`) and rides the existing degraded
path — a stuck version now holds the frontier, which is exactly what that alarm signals;
consumers meanwhile keep pinning the frontier's previous edge and resolve it **coherently**
(uniformly pre-change), never a mixed set.

**The frontier is materialized, not recomputed (normative, D-136, 2026-08-01 review fix).**
Pin-eligibility as stated above is a predicate over *every* subject row of *every* version, and
its prefix clause makes it recursive — evaluated literally on the read path it is a scan of the
delta store on a p95 < 100 ms budget, and the `pricing.readmodel.pin_eligibility_overdue` alarm
has nothing to evaluate at all. Therefore the frontier is a **stored per-tenant watermark**,
`pricing_pin_frontier(tenant_id, catalog_version, advanced_at)` (§3.7): the ReadModelProjector
advances it **in the same transaction** that sets the last outstanding `warm_completed` marker
of the frontier's next version in order, and only ever forward (a later version's completion
never advances it past a gap — that is the D-114 prefix, enforced by construction rather than
re-derived). It is the **only** definition of "the newest pin-eligible version": consumers read
it from the read-model contract (§3.3) and pin its value, the ≤ 5s lag rule is measured against
it, and the overdue alarm fires on its age. Nothing recomputes the predicate at read time.

A consumer pins **one** pin-eligible `CatalogVersion` for the duration of a resolution/rating
run and resolves the complete frozen view via `pricingSnapshotRef`; at pin time the pinned
version MUST NOT lag the frontier by more than 5s. There is **no** draft read
and **no** default substitution.

**Window facts are projected as intervals (normative, D-99; projected set + horizon per
D-121).** A plan subject's row carries, per canonical scope key, the key's `PriceWindow`
**intervals** and states (`[effectiveFrom, effectiveTo)` + `scheduled | active | expired`;
cancelled windows are not projected) and the derived **coverage end** — never a point-in-time
"is active" boolean. "Active at `t`" and the D-80 coverage horizon are therefore evaluated **at
read time** against frozen intervals, which is what makes the six sellability predicates
point-in-time evaluable from a pinned version (`inst-sg-pinned`) *and* what makes the
time-driven `scheduled → active` / `active → expired` transitions need no re-projection (a
frozen version never mutates). Only window **mutations** re-project, as publish units (§4.2).
**The projected row set (normative, D-121, 2026-07-31 review fix):** a plan-subject delta
projects **every price row of the plan — any lifecycle state except never-published drafts —
whose window set intersects `[projection_time − H, ∞)`**, each with its full interval set as
above, where **`H` = 2 × the longest billing cycle sold on the plan** (floor: one cycle + the
D-47 bulk batching SLO; tenant-configurable upward). The `expired` state and the horizon are
load-bearing: rating pins *current* versions and rates *past* instants (arrears always lag), so
after a changeover the superseded predecessor — its window now `expired` — must survive the
next re-projection or resolution at yesterday's `t` under the new delta fails closed on a
legitimately covered period. Anything older than `H` resolves by **replaying from the
originally-pinned version** (deltas are retained on the truth horizon, §3.7, so old pins stay
resolvable; posted periods never re-query per `fr-pricing-snapshot`). The bound is also the
size model: a delta is O(live keys × windows within `H`), never O(the plan's accumulated
history).

`pricingSnapshotRef` is the composite reference (`CatalogVersion` + resolved price ids +
evaluation-policy version) pinned on charges and `BillableItem`s — stamped at publish with the
**pending** version ref, finalized to the committed `CatalogVersion` on
`CatalogVersionPublished`, and immutable thereafter; posted invoice periods MUST NOT re-query
mutable catalog rows. The **normative composition SoR is Tariffs**; the catalog-side view is
the aligned entry and MUST NOT diverge from it. On read-model outage the read path **fails
closed** (never serves stale). After a degraded publish the warm **re-drive continues past the
SLO**; on completion it sets the warm-completion marker (the version becomes resolvable —
monotonicity unaffected) and clears the degraded mark, raising an operations alarm meanwhile;
no new event name is introduced — consumers observe completion via the marker.

The projection **source** is the plan's **current revision**'s own truth rows — the `(plan_id,
revision)` row whose `lifecycle_state` is `published` **or `retired`** (**D-128**, 2026-08-01
review fix: "the published revision" had no referent once retirement flipped the only one, so a
re-warm or degraded re-drive of a retired plan would have projected an empty plan subject and
broken rating resolution for exactly the in-flight subscribers D-51 keeps coverage for), its
revision-scoped child rows (D-83), and the published price rows — for the
initial warm and every re-drive alike; the projector **never reads the open draft revision**,
so a degraded-warm re-drive cannot leak draft edits into a frozen version (2026-07-30 review
fix). The revision's `lifecycle_state` is itself a **projected plan-subject field** — that is
what sellability predicate (4) reads at the pin (D-128). Versions store **per-subject deltas** (D-86, subject-typed by D-91): resolving `(pin, subject)`
reads the subject's greatest completed version ≤ the pin — monotonicity is unaffected, completed
versions never mutate, and retention follows the truth-history horizon (§3.7); overlay and
membership publish units resolve through their own subject rows, never through a plan's. The
versioned read model carries **no operator-plane state** (D-85): drift/divergence flags live in
`pricing_operator_flag` and never appear in a frozen version.

One reference is deliberately **not** version-pinned: a **`migrated-origin`** snapshot (Slice 11,
D-87) is **self-contained** — synthesis materializes the complete evaluable row content into the
frozen payload, and consumers evaluate from that payload without resolving its ids through the
read model (a tier-2 reference row exists in no `CatalogVersion` by construction, D-76; a tier-1
row's historical instant predates any useful pin). Because it resolves through **no** version, it
needs its own read surface, which the read-model contract does not provide: consumers fetch it
from `GET /bss-pricing/v1/migrated-origin-snapshots/{subscriptionRef}` (Slice 11 §5, `plan × read`
service identity — **D-102**, 2026-07-31 review fix; registered as an inbound lane of the Tariffs
contract, [`../PRD.md`](../PRD.md) §9.2). Everything else resolves version-pinned as above.

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) — §2.2 (canonical scope key), §6.2/§6.7 (model kind, publish, events), §6.8 (versioning/immutability/supersession), §6.9 (consumer resolution), §9 (interfaces), §17.4 (validation rules), §17.5 (change mechanisms + `CatalogVersion` increment)
- **DESIGN**: [`../DESIGN.md`](../DESIGN.md) — canonical index (slice map, dependency order, cross-cutting statements)
- **ADRs**: [`../ADR/`](../ADR/) — `cpt-cf-bss-pricing-adr-canonical-scope-key`, `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`, `cpt-cf-bss-pricing-adr-pricewindow-consolidation`

**Traces to**: `cpt-cf-bss-pricing-fr-publish-validation-failclosed`,
`cpt-cf-bss-pricing-fr-published-rows-append-only`, `cpt-cf-bss-pricing-fr-plan-versioning`,
`cpt-cf-bss-pricing-fr-supersession`, `cpt-cf-bss-pricing-fr-pricing-snapshot`,
`cpt-cf-bss-pricing-fr-consumer-readmodel-resolution`,
`cpt-cf-bss-pricing-fr-catalogversion-increment`,
`cpt-cf-bss-pricing-fr-publish-fanout-atomicity`, `cpt-cf-bss-pricing-fr-event-contract`,
`cpt-cf-bss-pricing-fr-price-amount-validation`, `cpt-cf-bss-pricing-fr-mutation-idempotency`,
`cpt-cf-bss-pricing-fr-concurrent-edit`

Grouped by theme:

- the aggregate fail-closed validation pipeline (`fr-publish-validation-failclosed`)
- append-only history + versioning/supersession (`fr-published-rows-append-only` / `fr-plan-versioning` / `fr-supersession`)
- snapshot + monotonic read model (`fr-pricing-snapshot` / `fr-consumer-readmodel-resolution`)
- `CatalogVersion` request, degraded handling, frozen event set (`fr-catalogversion-increment` / `fr-publish-fanout-atomicity` / `fr-event-contract`)
- money/precision, idempotency, optimistic concurrency (`fr-price-amount-validation` / `fr-mutation-idempotency` / `fr-concurrent-edit`)
- the two primary API contracts (`cpt-cf-bss-pricing-interface-authoring-publish` / `cpt-cf-bss-pricing-interface-catalog-read-model`)
