<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md | Owners: BSS Product Catalog team -->

# DESIGN — Registry Foundation (Slice 1)

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
  - [Create a Product](#create-a-product)
  - [Define a SKU](#define-a-sku)
  - [Save an edit (draft or published head)](#save-an-edit-draft-or-published-head)
  - [Discard a never-published draft](#discard-a-never-published-draft)
  - [Publish an entity (the mechanics half)](#publish-an-entity-the-mechanics-half)
  - [Transition an entity (state-machine floor)](#transition-an-entity-state-machine-floor)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Validation pipeline](#31-validation-pipeline)
  - [3.2 Idempotency store](#32-idempotency-store)
  - [3.3 Error taxonomy (Foundation-owned codes)](#33-error-taxonomy-foundation-owned-codes)
  - [3.4 Concurrency doors (PRD §6.13 residents of this slice)](#34-concurrency-doors-prd-613-residents-of-this-slice)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
  - [4.1 `products_product`](#41-products_product)
  - [4.2 `products_sku`](#42-products_sku)
  - [4.3 `products_entity_version` (published history)](#43-products_entity_version-published-history)
  - [4.4 `products_idempotency`, `products_audit_log`, `products_outbox`](#44-products_idempotency-products_audit_log-products_outbox)
  - [4.5 Foundation-owned events](#45-foundation-owned-events)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

The Foundation is the shared engine every products-gear capability publishes through: the
`Product`/`SKU` entity model and identity rules (server-minted UUIDs, atomically reserved
`skuCode`), the two version counters (internal revision vs published version), the lifecycle
state machine core (`draft → published ↔ deprecated → retired`, `draft → discarded`,
forward-only), the fail-closed **registered-validator pipeline**, append-only published-version
history with diff, per-row optimistic concurrency (`If-Match` on the internal revision),
tenant-scoped idempotency, the broker-native event fan-out through a transactional outbox
(P-D-01), and the append-only audit trail — which under **P-D-21** records only what emits no
event: refusals, reads under elevation, and committed acts the design declares emit no broker
event.

The Foundation deliberately owns **no capability policy**: it does not know what a `PlanTier`,
a metering unit, a materiality threshold, or a freeze participant is. Capability slices author
draft state through Foundation write doors, register their validation rules and their material
field sets, contribute read-model fields, and call the Foundation publish API. Governance
(slice 05) is a **registered gate** inside the publish door, not a separate path around it.

### 1.2 Purpose

Give the eleven capability slices one set of doors with the invariants already paid for:
identity that cannot be reissued, versions that cannot be rewritten, transitions that cannot be
invented, retries that cannot duplicate, events that cannot be lost before they are
acknowledged, and rejections that always carry an audited reason. Per P-D-02, every human gate attaches to the entity-publish act.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Authors drafts through the write doors; publishes (via the slice-05 gate) |
| `cpt-cf-bss-products-actor-catalog-admin` | Same doors, wider grants; discard, deferred operations |
| `cpt-cf-bss-products-actor-events-audit` | Receives the outbox fan-out; owns transport (delivery/retry/DLQ) |
| `cpt-cf-bss-products-actor-oss-ams-idp` | Supplies `tenantId`, brand/region claims, roles; the registry never mutates tenant topology |

### 1.4 References

- [`../PRD.md`](../PRD.md) §2.1, §6.1, §6.5, §6.7, §6.8, §6.13, §9, §10, §15, §17.1; AC #1, #2 (frame), #5 (uniqueness), #13,
  #14, #27, #28, #38, #42
- [`../DECISIONS.md`](../DECISIONS.md) P-D-01 (envelope), P-D-02 (mechanical increments),
  P-D-04 (absolute name uniqueness), P-D-06 (metadata-map placement), P-D-08 (audit-sealing
  seam), P-D-21 (the audit table holds only what emits no event)
- Pricing `design/01-foundation.md` — the pattern donor (registered validators, append-only
  triggers with column whitelists, draft/published partial unique indexes, outbox + pending
  refs); divergences are stated where they occur
- `gears/system/event-broker/docs/DESIGN.md` + its ADR-0003 — the envelope this gear emits

### 1.5 Scope

**In**:
- entity model + storage shape for `Product`/`SKU` core columns
- identity + `skuCode` reservation
- revision/version mechanics
- state-machine core (edge list, terminality, physical floor)
- validation pipeline frame + the error taxonomy
- the publish door and the governance gate's host contract (`Gate` / `PreAuthorized(approvalId)`)
- field-mutability enforcement frame (bucket routing)
- idempotency
- ETag concurrency
- outbox + envelope discipline + per-aggregate ordering
- audit of the acts that emit no event (refusals; reads under elevation; committed acts declared
  to emit no broker event — P-D-21)
- the interim parent-child brand/region containment check (P-D-04 residue — final rule in slice 04)
- the reserved platform audit-sealing seam's columns, CHECK and one-way trigger (P-D-08 — present
  from the first migration, never sealed here; §4.4).

**Out** (owned by later slices, listed so absence reads as intent):
- category/attribute content rules (02)
- typing/classification/metering policy (03)
- deprecation/retirement policy, scheduling, cascades (04)
- materiality, approvals, RBAC grants, break-glass (05)
- `CatalogVersion` and freezes (06)
- `SkuReferenceCount` and corrections (07)
- read models (08)
- bulk (09)
- retention/erasure execution (10)
- clone (11)
- seam suite + replay/bootstrap (12).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Dual-engine storage (SQLite + Postgres); one migration per table, guards defined once; schema oracle goldens from day one | house practice (pricing migration-chain rebuild) |
| C2 | Broker-native envelope; no CloudEvents field anywhere in the payload path | P-D-01 |
| C3 | No money, no price, no charge computation in this gear | PRD §2.1 |
| C4 | Every table carries `tenant_id`; all repository access through SecureORM tenant scoping | PRD §6.8; ToolKit |
| C5 | Append-only posture: head rows and history rows are physically guarded (REVOKE + trigger whitelist), not just conventionally. Exempt by design: the slice-08 projection family (rebuildable state, not records) and expiring operational stores (idempotency sweep) | PRD `fr-revision-vs-version` |
| C6 | Idempotency-key retention ≥ 24h **and** ≥ the maximum freeze timeout (slice 06 exports the number; the store reads it as config) | PRD AC #27 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `RegistryEntity` | The trait both `Product` and `SKU` implement toward the pipeline: kind, id, tenant, lifecycle state, internal revision, published version |
| `ValidationPipeline` | The ordered, fail-closed run of shape → identity → registered slice validators → governance gate (publish only, §3.1), executed inside every mutating door |
| `RegisteredValidator` | A slice-contributed rule keyed by `(entity kind, transition or field set)`; registration is code, not config |
| `PublishDoor` | The single Foundation API that turns an approved draft into a published version (bump, snapshot, events) — the only writer of `published_version` |
| `ReservationIndex` | The partial unique index realizing `skuCode` reservation (see §4.2) |
| `identity-reference map` | The pseudonym → operator identity table audit/events point at; its erasure semantics are slice 10's |

### 1.8 Context & Dependencies

**Consumed**: IdP claims (tenant/brand/region/roles); event-broker SDK (publish, durable ack);
platform config store (interim policy defaults, PRD §17.1). **Produced**: Foundation events
(§4.5), which under P-D-21 are the success-path audit record; refusal, elevation and declared-no-event audit rows;
the SDK read/write surface the studio and sibling gears call. **Explicitly
not consumed here**: `SkuReferenceCount` (slice 07), pricing signals (06/07).

## 2. Actor Flows (CDSL)

### Create a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-create-product`

1. [ ] - `p1` - Authorize `product × write` in the caller's tenant/brand scope (deny-by-default); resolve the idempotency key `(tenant, endpoint, client key)` — a hit with identical payload replays the stored outcome, a hit with different payload fails `IDEMPOTENCY_CONFLICT` - `inst-fd-idempotency`
2. [ ] - `p1` - Validate shape; normalize `name`; enforce **absolute** uniqueness on `(tenant_id, brand_id, name_normalized)` via the partial unique index (§4.1) — collision fails `DUPLICATE_NAME` naming the holder; P-D-04: region scope plays no part - `inst-fd-name-unique`
2a. [ ] - `p1` - Resolve the acting principal to its `actor_ref` through slice 10's
`products_identity_ref`, in the door's own transaction, minting on a principal's first appearance
— 10 `inst-im-map` states the obligation from its side ("01's doors mint refs through it") and this
slice never carried it, while `created_by`, the frozen version's `actor_ref`, the audit row's and
the envelope's all store the result. Resolving also advances `last_seen_at`, which 10's age-based
erasure reads - `inst-fd-actor-ref`
3. [ ] - `p1` - Mint `productId` (UUID, server-side, never caller-supplied — a stray id in the payload is a `400`); optional `productCode` reserves under the same rules as `skuCode` - `inst-fd-mint-id`
4. [ ] - `p1` - Persist as `draft`, `published_version = 0`, `internal_revision = 1`; write the `ProductCreated` outbox row in the same transaction (**P-D-21**: the event is the
success-path audit record; no audit row is written on a committed act) - `inst-fd-create-txn`

### Define a SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-define-sku`

1. [ ] - `p1` - Authorize; idempotency as above - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - Parent must exist in the tenant, not be `retired`/`discarded`, **and not hold a live retire intent** (`RETIREMENT_PENDING` — item 36 of the 2026-08-26 review: a `deprecated` parent still admits children, so a draft SKU created after the `CascadePlan` was computed is outside the plan's auto-discard arm and defers that Product's retirement indefinitely); the SKU's brand/region scope must pass the **interim containment check**: scope sets are flat value lists, containment = subset, anything not provably a subset fails `SCOPE_NOT_CONTAINED` (conservative until slice 04 pins the final rule) - `inst-fd-containment`
3. [ ] - `p1` - Reserve `skuCode` **atomically at create**: the insert itself is the reservation — the `ReservationIndex` admits exactly one non-`discarded` holder per `(tenant_id, sku_code)`; the loser of a concurrent race fails `DUPLICATE_SKU_CODE` with an audited reason (PRD AC #42) - `inst-fd-reserve-code`
4. [ ] - `p1` - Mint `skuId`; persist as `draft` with the slice-03-owned columns present but unjudged (typing/classification rules run when slice 03 registers them); emits the `SkuCreated` outbox row in the same transaction - `(cont. inst-fd-create-txn)`

### Save an edit (draft or published head)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-save-draft`

0. [ ] - `p1` - Authorize; idempotency as at create - `(cont. inst-fd-idempotency)`
1. [ ] - `p1` - Every mutating verb on an entity head **requires `If-Match`** on the internal revision; mismatch fails `STALE_REVISION`; an absent precondition is a malformed request (per-row token, never plan-shared — the pricing D-141 lesson adopted at birth) - `inst-fd-etag`
2. [ ] - `p1` - Run the pipeline's shape + identity phases plus every registered validator for `(kind, field set)`; violations collect per-field into one audited rejection - `inst-fd-pipeline`
3. [ ] - `p1` - Saves land on the **head row** — the authoring surface for `draft`, `published`, and `deprecated` entities alike (H1 fix, 2026-08-25 review): a save is never a lifecycle transition, and consumers are untouched because **every consumer-facing read of Product/SKU entity content serves frozen `products_entity_version` content, never the head row**. A `skuCode` change is legal only while `published_version = 0` and releases the old code by the row update itself; `internal_revision += 1`; the `ProductDraftSaved`/`SkuDraftSaved` outbox row in the same transaction. Saves **never** touch `published_version` - `inst-fd-save-txn`
4. [ ] - `p1` - A draft save on an entity holding an open approval **invalidates it** — the Foundation raises the `approval-invalidated` hook; slice 05 owns re-queue semantics - `inst-fd-approval-hook`

### Discard a never-published draft

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-discard`

1. [ ] - `p1` - Legal only from `draft` with `published_version = 0`; transition to `discarded` (terminal); the `ReservationIndex` (§4.2) and the `product_code` index (§4.1) both exclude `discarded` rows, so the `skuCode`/`productCode` reservation releases by the same write; emits the `SkuDiscarded`/`ProductDiscarded` event - `inst-fd-discard`

### Publish an entity (the mechanics half)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-publish`

0. [ ] - `p1` - Authorize; idempotency as at create — the PRD names **publish** among the retried
verbs, and 05's crash-replay of a scheduled activation rides this store keyed by transition id - `(cont. inst-fd-idempotency)`
1. [ ] - `p1` - `PublishDoor` accepts `(entity, expected internal revision)` — a `draft` for its first publish, or a `published`/`deprecated` **head** for version N+1 (a re-publish changes the version, never the state); stale revision fails `STALE_REVISION` — an approval is only usable against the exact revision it pinned (slice 05 stores the snapshot; the Foundation enforces the match) - `inst-fd-publish-pin`
2. [ ] - `p1` - Re-run the **full** pipeline at publish (shape, identity, every registered validator for the `→ published` transition): an entity that stopped being publishable since approval fails closed `INCOMPLETE_ENTITY`/rule-named code, never publishes stale - `inst-fd-publish-revalidate`
3. [ ] - `p1` - The governance gate (slice 05) runs **inside** the door, and the door therefore carries an explicit **authorization mode** (Blocking 9 fix, 2026-08-26 review): `Gate` — the ordinary interactive publish, which needs a `satisfied` record — or **`PreAuthorized(approvalId)`**, the mechanical stage of a composite act (05 `inst-gv-one-shot`: scheduled activation, a cascade leg, a bulk row). Under `PreAuthorized` the gate does not look for a `satisfied` record and does not consume one; it **verifies** that the named record authorized *this* subject and that its pinned revision still matches, raising `APPROVAL_REQUIRED` only when it did not. Without the mode the two readings collide and every scheduled publish fails terminally: the runner drives "the ordinary Foundation publish door" (04 `inst-sp-activate`), the gate inside it would see a `consumed` record, and 04 `inst-ar-failure` wraps that into a terminal `SCHEDULE_STALE_APPROVAL`. Re-validation stays fail-closed in both modes — the mode governs *who approved*, never *whether the entity is still publishable*. A material change without satisfied approvals fails `APPROVAL_REQUIRED`; the Foundation knows only "the gate answered yes/no + reason". An approval rejection "returns the entity to draft" (AC #26) reads: a first-publish entity stays `draft`; a published head keeps its pending edits unpublished — no state flip either way (design reading under the head-row model, **flagged**: the literal reading would need a `published→draft` edge the PRD's own forward-only rule forbids — slice-05 review L-6) - `inst-fd-governance-gate`
4. [ ] - `p1` - On yes: `published_version += 1` (the door is this column's **only** writer); freeze the full entity content (excluding the metadata map, §4.3) into `products_entity_version`; first publish makes `skuCode` reservation permanent (immutability enforced by the trigger whitelist from this row-state on); emit `ProductPublished`/`SkuPublished`; mark the gate's `satisfied` `ApprovalRecord` `consumed`
(05 `inst-gv-one-shot` requires the flip **in the same transaction as the authorized act**;
nothing is consumed under `PreAuthorized`) — all in one transaction - `inst-fd-publish-txn`
5. [ ] - `p1` - Post-commit, slice 06 consumes the publish event **as content only** (what became publishable); an entity publish **never enqueues a CatalogVersion increment** — addressability comes from downstream requests or an operator catalog-publish act (06 `inst-cv-request`; M1 fix of the 06 review), and the Foundation itself requests nothing (06 `inst-cv-request`'s trigger set names pricing, this gear's slice-09 bulk commits and the operator act — not this slice) - `inst-fd-publish-fanout`

### Transition an entity (state-machine floor)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-transition`

1. [ ] - `p1` - The edge list guards **`lifecycle_state` changes only** — a save is not a transition, and a re-publish is not an edge (H1 fix: the head row is the authoring surface in every non-terminal state). The Foundation admits exactly: `draft→published` (door), `draft→discarded`, `published→deprecated`, `deprecated→published`, `deprecated→retired`; anything else fails `ILLEGAL_TRANSITION`. **Every transition bumps `internal_revision` and fires the approval-invalidation hook** exactly as a save does (M-2 fix, 2026-08-25 slice-05 review: head-at-revision-N stays byte-identical to any approval snapshot pinned at N — transition-written columns cannot drift under a pin). Policy conditions on the legal edges (two-person on un-deprecate, scheduled retirement, cascades) are slice 04/05 validators registered on the edge — the floor stays policy-free - `inst-fd-transition-guard`
2. [ ] - `p1` - `retired` and `discarded` are terminal at the physical layer too: the append-only trigger's whitelist admits no `lifecycle_state` write out of them - `inst-fd-terminal`

## 3. Processes / Business Logic

### 3.1 Validation pipeline

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-pipeline`

1. [ ] - `p1` - Ordered phases: **shape** (types/formats/required-at-this-state) → **identity** (uniqueness, reservation, containment) → **registered validators** (each slice contributes `RegisteredValidator`s keyed by kind + transition/field-set; execution order is registration order within a phase, and no rule may read another rule's verdict) → **governance gate** (publish only) - `inst-fd-pipeline-order`
2. [ ] - `p1` - Fail-closed and atomic: any failure rejects the whole mutation with an audited reason; there is no partial application anywhere in the gear (PRD AC #38) - `inst-fd-fail-closed`
3. [ ] - `p1` - Registration is compile-time code (a slice ships its validators with its handler); the pipeline exposes `rule_names()` for observability only — attribution in rejections rides the **error code**, never the rule name - `inst-fd-rule-registry`
4. [ ] - `p2` - Field-mutability enforcement frame: each published-state field carries a bucket tag (i structural / ii correctable / iii material-mutable / iv descriptive — PRD `fr-field-mutability-matrix`); the Foundation refuses bucket-i writes after first publish and routes bucket-ii to slice 07's correction door; bucket iii/iv are ordinary head-row saves re-published as version N+1, their materiality judged by slice 05; bucket-ii writes are admitted **only through slice 07's correction door**, which the physical guard (§4.2) knows by name - `inst-fd-mutability-frame`

### 3.2 Idempotency store

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-idempotency`

1. [ ] - `p1` - Key scope `(tenant_id, endpoint, client_key)`; stored: payload hash + outcome reference; identical replay returns the stored outcome without touching entities, versions, or the outbox; different payload under a live key fails `IDEMPOTENCY_CONFLICT` (never a silent no-op) - `inst-fd-idem-replay`
2. [ ] - `p1` - Retention: `max(24h, max_freeze_timeout)` read from config (C6); expiry is a background sweep; expiry never retro-invalidates an outcome - `inst-fd-idem-retention`

### 3.3 Error taxonomy (Foundation-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-error-taxonomy`

`DUPLICATE_NAME`, `DUPLICATE_SKU_CODE`, `STALE_REVISION`, `IDEMPOTENCY_CONFLICT`,
`ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION` (bucket-i write), `SCOPE_NOT_CONTAINED`,
`PARENT_NOT_PUBLISHED` (registered by slice 04 on the publish edge but named here so AC #38's
map is complete), `INCOMPLETE_ENTITY`, `APPROVAL_REQUIRED` (raised through the governance
gate), `VALIDATION` (per-field envelope), `RETIREMENT_PENDING` (the create door's parent guard,
`inst-fd-containment` — declared here 2026-08-26; slice 04 owns the un-deprecation arm of the
same code, so this code has two raising doors, and the
one-door rule below is stated per **arm** for it). Every code appears in exactly one raising
**door** — a door, not a slice: validators registered into the `PublishDoor` raise through it;
slice-owned codes (taxonomy cycles, unit rules, freeze, bulk rows…) are declared in their
slices and the AC #38 ↔ code ↔ slice map is completed by slice 12's coverage check. **Two
carve-outs are named here so that rule stays checkable**: `RETIREMENT_PENDING` has two arms
(this slice's create-door parent guard and slice 04's un-deprecation), and `CONTENT_PII_BLOCKED`
is raised by the shared `inst-av-pii-block` hook rather than by a door at all — every door
carrying a free-text `reason` invokes it, and slice 02 holds its single declaration. Codes are
part of the SDK contract; renames are breaking.

**Problem responses (RFC 9457):** `APPROVAL_REQUIRED` (403); `DUPLICATE_NAME`, `DUPLICATE_SKU_CODE`, `IDEMPOTENCY_CONFLICT`, `RETIREMENT_PENDING`, `STALE_REVISION` (409); `ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION`, `SCOPE_NOT_CONTAINED`, `PARENT_NOT_PUBLISHED`, `INCOMPLETE_ENTITY`, `VALIDATION`, `CONTENT_PII_BLOCKED` (422).

*Statuses added 2026-08-26, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, 2026-08-02, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a 2026-08-26 pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see the rendering note below, which quotes
the sibling plan-price gear's rule: no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for a **canonical** error in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `CONTENT_PII_BLOCKED`
  (slice 02) — the status is repeated, not a second declaration, so the one-declaration rule
  stands.*

**Status rendering — the 422s in this set are architectural, not wire (normative).**

The `422` annotations in every slice's problem-response block say *unprocessable content*: the
request was understood and the registry refuses it. They are **not** a wire status. The platform's
`CanonicalError` model has no 422 category — `libs/toolkit-canonical-errors/src/problem.rs` carries
no 422 arm — so, absent a transport override (which neither this design set nor pricing's
declares anywhere), every architectural 422 in this design set reaches the wire as a **400
carrying its wire code**, and **the code string is the discriminator a consumer matches on, not the status**.
An endpoint **MUST NOT** declare a 422 response for an error **carrying a registry code** in its
`OpenAPI` registration — **the rule stands; its grounds are flagged in §6** (2026-08-27). It read
"because no path can produce one" until this pass, and that is false: 400 is the *default* for
`InvalidArgument`/`FailedPrecondition`/`OutOfRange`, and `docs/arch/errors/DESIGN.md` §2.2 lets a
single occurrence override the wire status so long as it stays in the same class — 422 is in that
class, and the toolkit's own `extract::Json` path takes it. **The framework layer is the exception and is not covered by it**: a handler taking
`toolkit::api::rest::extract::Json<T>` can still answer 422 on a schema violation, which is why the
toolkit ships `OperationBuilder::error_422` and tells an operation to register it individually
(`libs/toolkit/src/api/operation_builder.rs`). A schema violation carries no registry code — though it **is** a canonical error: the toolkit
builds one (`json_rejection_to_canonical`, `libs/toolkit/src/api/rest/extract/error.rs`) and
renders it 422 as the canonical `invalid_argument` category — on the wire the `type` is
`gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~`, the symbol naming it in the
toolkit being test-only. The **quotation below** is the sibling plan-price gear's rule verbatim; the canonical scoping and the framework exception above are this gear's own, added 2026-08-27 because pricing states the rule unscoped and the toolkit contradicts the unscoped form, and it is quoted rather than
paraphrased: `gears/bss/pricing/docs/design/01-foundation.md` §3.3 — *"The platform's
`CanonicalError` model has **no 422 category** at all (`InvalidArgument`, `FailedPrecondition` and
`OutOfRange` all render **400**), so every architectural 422 in this design set — here and in every
slice — reaches the wire as a **400 carrying its wire code**"*. Two consequences bind the implementation, the first from the rule quoted above and the second
from the same section's pagination rule, whose subject there is an undecodable cursor — *"a malformed request … answered 400 **with no code of its
own**"*: a refusal is classified by what it **is**, so a retriable
conflict on mutable state stays a **409** rather than collapsing into the 400 bucket; and a bare
**400 with no code of its own** is reserved for a malformed request, which is why no registry code
is mapped to 400. Stated here once, in the Foundation, rather than per occurrence.

### 3.4 Concurrency doors (PRD §6.13 residents of this slice)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-concurrency`

1. [ ] - `p1` - `skuCode` race: decided by the `ReservationIndex` under the insert, not by a read (two concurrent creates: exactly one admitted) - `inst-fd-code-race`
2. [ ] - `p1` - Draft race: decided by `If-Match` (two editors: second fails `STALE_REVISION`) - `(cont. inst-fd-etag)`
3. [ ] - `p1` - Publish-vs-edit race: the door's pinned-revision check makes "approve rev N, publish rev N+1" impossible by construction - `(cont. inst-fd-publish-pin)`

## 4. Data / Storage (normative shape; DDL in migrations)

### 4.1 `products_product`

`product_id` (PK, uuid) · `tenant_id` · `brand_id` · `name` · `name_normalized` ·
`product_code` (nullable) · `lifecycle_state` (`draft|published|deprecated|retired|discarded`) ·
`internal_revision` · `published_version` · `deprecation_provenance` (nullable
`direct|cascaded`, slice 04) · **category assignments live ONLY in slice 02's `products_product_category`** (the assignment
table with the exactly-one-primary partial index — a second inline representation here would
be a divergence channel with no authority rule; the frozen version content carries the
assignment set as a copy at publish, like every other content class) · `region_scope` /
`brand_scope` · `created_by` (pseudonymous ref) · `cloned_from` (create-only, immutable —
slice 11) · timestamps.

Indexes/guards: **partial UNIQUE `(tenant_id, brand_id, name_normalized) WHERE lifecycle_state
<> 'discarded'`** (P-D-04; discard releases the name exactly as it releases codes — a
design-introduced symmetry, flagged in §6); partial UNIQUE on `(tenant_id, product_code) WHERE
product_code IS NOT NULL AND lifecycle_state <> 'discarded'`; append-only trigger enforcing the
shared head-row guard (§4.2), under which `product_code` is immutable once
`published_version > 0` exactly as `sku_code` is (PRD AC #1 puts an optional `productCode` under
the same rules as `skuCode`, and the guard named only the SKU column).

**`normalized(name)`** (the uniqueness and promotion-identity operand, P-D-04/AC #33a) is pinned:
Unicode NFKC → full casefold → trim + collapse internal whitespace to single spaces, computed
**application-side** so both engines store identical bytes.

### 4.2 `products_sku`

`sku_id` (PK, uuid) · `tenant_id` · `product_id` (FK) · `sku_code` · `type`
(`product|service|bundle`) · `lifecycle_state` · `deprecation_provenance`
(nullable `direct|cascaded`, slice 04) · `sellable` (default `true`, pricing D-46) · `plan_tier` ·
`tax_category_ref` · `gl_code_ref` · `metering_unit` · `usage_type_ref` ·
`composition_pending` (bool, slice 06 semantics) · `replaced_by_sku_id` (slice 04) ·
`internal_revision` · `published_version` · `region_scope`/`brand_scope` (⊆ parent, §2 flow) ·
`created_by` · `cloned_from` (nullable; written only at create, immutable after — a later write
fails `ILLEGAL_FIELD_MUTATION`; slice 11) · timestamps.

Column ownership: the Foundation owns identity/lifecycle/version/scope columns; every
capability column above is **carried** here (one table, one row identity) but its write rules
are the owning slice's registered validators — the split is by validator, not by table.
**`ReservationIndex`: partial UNIQUE `(tenant_id, sku_code) WHERE lifecycle_state <>
'discarded'`** — atomic reservation at insert, release on discard, permanence after first
publish enforced by the trigger whitelist making `sku_code` immutable once
`published_version > 0`.

**Shared head-row guard (both entity tables; H1/M1 fix, 2026-08-25 review):** frozen
`products_entity_version` rows admit no UPDATE/DELETE ever. On head rows the trigger whitelist
admits exactly: `lifecycle_state` along the §2 edge list; `published_version` only from the
`PublishDoor`; bucket-iii/iv columns via the save door in any non-terminal state;
`internal_revision` via the save door **and from every transition and correction door — every
one of them bumps it** (`inst-fd-transition-guard`), so admitting it "via the save door" alone
refused writes the design requires; **`deprecation_provenance` and `replaced_by_sku_id` only
from slice 04's own doors**, and **`composition_pending` only from slice 06's clearing lane
(`inst-cc-clear`), never the save door — 06 declares the flag system-owned and never
operator-mutable, so bucket iii/iv would be the wrong home** (deprecation/cascade and
retirement-initiation respectively — they
are neither save-door nor bucket-iii/iv columns, and leaving them unnamed either refused the
writes slice 04 specifies or dropped them to bucket iv, where an ordinary operator save could
re-stamp the provenance operand `inst-lc-provenance-reversal` reads — item 18 of the 2026-08-26
review); bucket-ii columns only via the slice-07 correction door; bucket-i identity columns
never after first publish.

### 4.3 `products_entity_version` (published history)

`(tenant_id, entity_kind, entity_id, published_version)` UNIQUE · frozen full content **excluding the metadata map** (slice 02's `products_metadata`; P-D-06 — the map lives beside the entity,
captured only by `CatalogVersion` snapshots) — the publish-time entity, engine-canonical
serialization, the byte-identity discipline that `CatalogVersion` (slice 06) will reuse ·
**a per-row content digest written at freeze** (the slice-10 restore drill re-verifies sampled
entity versions against it — without it, version-history corruption is invisible to every
checksum; H2 fix) · `approval_ref` · `actor_ref` · `published_at`.
Append-only, no UPDATE path at all; diffs are computed between rows, never stored mutated.
These rows are the **only consumer-read surface** for **Product/SKU** entity content (08 C6's
own scoping — governed live entities are read from their live tables): read models,
`CatalogVersion`, and the SDK's consumer-facing reads project from here — never from head rows.
The authoring read of the head row that `inst-fd-etag`'s precondition requires is not a consumer
read.

### 4.4 `products_idempotency`, `products_audit_log`, `products_outbox`

- `products_idempotency`: `(tenant_id, endpoint, client_key)` PK · `payload_hash` ·
  `outcome_ref` · `expires_at` (§3.2).
- `products_audit_log`: append-only apart from the one-way seal transition below; `actor_ref` (pseudonymous — the identity-reference map is
  slice 10's), action, subject `(kind, id, revision)`, reason, correlation id, `written_at` (the
  operand `RetentionClock`'s audit class reads — 10 §3), and a nullable break-glass
  `session_id` (present on the elevation class only; 05 audits every elevated access with it). **Under P-D-21 this
  table holds only the acts that emit no event**, three classes: every **refusal** with its reason
  (the cases of `fr-expected-failure-behavior` a registry door can refuse — 12 `inst-cc-errors`
  puts three of the fifteen outside that universe so the lint is buildable); every **read under elevation** with its
  break-glass session id (05 — elevation in v1 is audit-export only, so nothing under it commits a
  mutation and nothing under it produces an event); and every **committed act the design declares
  emits no broker event** (04's `PublishScheduled`/`RetirementScheduled` — "audit-plane records,
  explicit \"no broker event\" per 01 §4.5" in that slice's own words. Slice 10's erasure act is
  **not** in this class: it is eventless only for events *carrying identity*, and its own
  **Produced** set lists a minimal `ActorErased(actor_ref)`). A committed mutation that *does* emit writes
  **no** row here; its outbox event is the record. **How a refusal's row commits is unsettled, and
  P-D-21 makes it load-bearing** (§6): the wording this replaced had every door write its row
  "in its transaction", which is precisely the transaction a refusal rolls back.
  **Reserved platform-sealing seam (P-D-08)** — present from the first migration, never sealed
  by this gear: `seal_state` (NOT NULL, roster `unsealed | sealed`, written **`unsealed` at
  INSERT** by this gear — always, in v1 and after activation alike — which makes the unproven
  era queryable instead of inferred from a deployment date; **the trigger whitelist admits
  exactly one UPDATE on this column group: a one-way `unsealed → sealed` transition supplying
  `chain_id`/`seq`/`prev_hash`/`row_hash` in the same statement, under the platform sealer's own
  identity,
  never on a row already `sealed`** — without which the seam is not activatable at all, since
  P-D-08's S3 computes the seal asynchronously **over rows already immutable**, so `row_hash`
  does not exist at INSERT and the CHECK refuses an INSERT as `sealed` while an
  outside-the-whitelist column refuses the async write too, leaving exactly the migration the
  seam exists to avoid — item 7 of the 2026-08-26 review) plus `chain_id` · `seq` ·
  `prev_hash` · `row_hash`, all nullable. One CHECK ties them so no half-populated row can
  exist: `unsealed` ⇒ all four NULL; `sealed` ⇒ `chain_id`/`seq`/`row_hash` NOT NULL
  (`prev_hash` NULL stays legitimate — it is the segment head). The gear computes no hash and
  runs no verification job; what the platform capability must satisfy is P-D-08 S1–S9.
- `products_outbox`: event rows written in the mutation transaction; `(tenant_id, aggregate_id,
  sequence)` monotonic per aggregate, **enforced by a UNIQUE `(tenant_id, aggregate_id, sequence)`
  allocated inside the mutation transaction** — the donor names the same mechanism and records
  that it had to, the ordering having been asserted with nothing holding it; dispatcher publishes to the event-broker and marks
  delivered **only on durable broker acceptance** — "emitted" is never reported before that
  (PRD `fr-event-delivery-resilience`, registry-side half). Payloads: broker-native envelope
  (P-D-01) with versioned schema ref, correlation/causation, idempotency key, `actor_ref`.

### 4.5 Foundation-owned events

`ProductCreated`, `SkuCreated`, `ProductDraftSaved`, `SkuDraftSaved`, `ProductPublished`,
`SkuPublished`, `ProductDiscarded`, `SkuDiscarded`. Rule for every other slice: each
state-changing instruction names its event or records "no event" in its slice doc — the
completeness check is slice 12's. Schema versioning (`vN`→`vN+1`) discipline and the
replay/bootstrap path are specified in slice 12; the Foundation's obligation is the envelope
and the ordering key.

## 5. Testing posture (slice-local)

- Schema oracle from day one (C1): canonical dumps of both engines golden-frozen; a
  perturbation case proving the oracle can fail.
- Every refusal in §2/§3 paired with a positive control (the fixture-grants lesson); the
  `ReservationIndex` race and the publish-pin race get real concurrency probes, not
  read-then-assert.
- The trigger whitelist gets a `CorruptRow`-style probe per guarded column class (poison
  columns are the missing guards).
- No `#[ignore]`d tests without a CI tier that runs them.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-product-sku-editor` (§10 use case, claimed by id here 2026-08-26 — all seven were in lint 1's universe and none was claimed); **NFRs by id** — #3 `cpt-cf-bss-products-nfr-publication-propagation` (the outbox half of the < 3 s event-availability budget: `DESIGN.md` §1.2's NFR Allocation pairs two budgets with two mechanisms — "the < 3 s propagation and < 5 s posting-safe budgets on the slice-01 outbox + slice-06 freeze machine" — and the PRD calls this one "a component **preceding** freeze acks", so it is the outbox, the freeze machine's budget being `nfr-posting-safe-budget`. **The probe is owed**: no slice §5 measures it. **And the split is unsettled** — slice 06 claims a freeze-machine half of this same budget, `DESIGN.md` §1.2's coverage table assigns the id to **both** slices, and `PRD` calls the < 3 s budget "a component **preceding** freeze acks", which reads against a freeze-machine share. Registered in slice 06's open items; the owed probe is what would settle it), #6 `cpt-cf-bss-products-nfr-scale-extensibility` (the entity-count half: the head/version split and the index shape; `CatalogVersion` growth is slice 06's), #8
`cpt-cf-bss-products-nfr-determinism-integrity` (version immutability, taxonomy acyclicity, identity uniqueness and
metering-unit validity enforced fail-closed: this slice's pipeline, edge list and trigger
whitelist are the frame its registered validators run in — acyclicity is slice 02's rule set,
metering-unit validity slice 03's, the posted-period snapshot clause slice 06's — and the id was
referenced nowhere in the set until item 30 of the 2026-08-26 review); **§9 by id** — `cpt-cf-bss-products-interface-authoring-publish` (§9.1: this slice owns the authoring and publish doors, idempotency keys and `If-Match`; the id's
intent-declaration clause is slice 06's `inst-rv-intent` door, and slice 12 carries the idempotency keys, `If-Match` and intent semantics into the SDK
contract) and `cpt-cf-bss-products-contract-registry-events` (§9.2 outbound: the broker-native envelope + outbox fan-out are this slice's). *(§9 ids were claimed by no slice's Traces-to at all until the 2026-08-26 branch review — prose like "§9 (all **seven** id-bearing blocks across §9.1/§9.2 …)" is not the id lint 1 keys on, so it would have reported zero claims for the whole §9 surface on its first run. Exactly the item-30 defect, left standing for §9 by the wave that widened the lint to include it.)* `cpt-cf-bss-products-fr-identifier-contract`, `cpt-cf-bss-products-fr-create-product` (uniqueness carrier),
`cpt-cf-bss-products-fr-define-sku` (identity carrier), `cpt-cf-bss-products-fr-revision-vs-version` (counters + history halves; the
version-binding-at-freeze clause → slice 06), `cpt-cf-bss-products-fr-lifecycle-transitions`
(machine core), `cpt-cf-bss-products-fr-idempotent-authoring`, `cpt-cf-bss-products-fr-registry-eventing-audit` (envelope + outbox
half), `cpt-cf-bss-products-fr-event-delivery-resilience` (registry-side half: durable acceptance + outbox),
`cpt-cf-bss-products-fr-parent-child-integrity` (interim containment half; final rule → slice 04),
`cpt-cf-bss-products-fr-skucode-reservation-concurrency`, `cpt-cf-bss-products-fr-field-mutability-matrix` (enforcement frame),
`cpt-cf-bss-products-fr-expected-failure-behavior` (taxonomy home); AC #1, #2 (mutability frame), #5 (name-uniqueness half), #13, #14, #27, #28 (envelope), #38
(frame), #42.

**Risks & open items**:
- **Name release on discard** is design-introduced symmetry (PRD releases *codes* on discard,
  says nothing about the name) — cheap to revisit; flagged for the slice review.
- **Clone-of-retired vs absolute name uniqueness** (P-D-04): a retired product holds its name,
  so revival-by-clone forces a rename — **resolved by slice 11 `inst-cn-rename`** (canonical
  name renames, display attributes copy verbatim; a name-*transfer* would be a P-D-04
  amendment, deliberately not built).
- **Broker schema-version pinning**: the versioned-schema-ref mechanics on the broker side need
  one worked example with Common Core before slice 12 freezes the replay contract.
- **`sellable` member missing in pricing's `CatalogSku`** — pricing-side gap (2026-08-25
  review); the SDK read shape here must expose it so the fix is a consumer-side addition.
- Interim containment check (flat subset) must be re-validated against slice 04's final rule —
  the two must not silently diverge.
- **`SCOPE_NOT_CONTAINED`: one raising door or two?** (both lenses of the 2026-08-27 slice-01
  review, independently). §3.3 states the one-door rule with a **closed** list of two carve-outs,
  yet this slice raises the code at the create door (`inst-fd-containment`) and slice 04 raises it
  on save and publish (`inst-pc-containment`, which 04 §1.6 calls "the final form of 01's interim
  check"). 04's own note covers *declaration* only — "the status is repeated, not a second
  declaration" — and says nothing about the raising **door**, which is the unit this rule is
  stated over. Either a third carve-out is owed, or 01's create-door arm retires when 04 lands.
  Both are product calls; neither is written anywhere.
- **Who reads the retire intent at the create door?** `inst-fd-containment` refuses a child under
  a parent holding a live retire intent, but §1.1 gives the Foundation **no capability policy**,
  §1.5 puts retirement scheduling in slice 04, and no table in §4 can express an intent — the
  operand exists only as 04's `products_scheduled_transition` and `CascadePlan`. Whether this is a
  registered slice-04 validator on the create door or a Foundation read of 04's store is unstated,
  and the two differ in who owns the coupling.
- **What column carries outbox delivery state, and is its UPDATE inside C5's guard?** §4.4 has the
  dispatcher mark delivered **only on durable broker acceptance**, but the `products_outbox`
  roster defines no such column and C5 exempts only the slice-08 projections and the idempotency
  sweep. Slice 10's restore scope depends on the state ("outbox-delivered"). The pattern donor
  names no column either (pricing `design/01-foundation.md`'s `pricing_outbox` row), so naming one
  here would be inventing schema no document states.
- **Why is the field-mutability frame `p2`** when the physical guard enforcing its buckets is
  unconditional and its code `ILLEGAL_FIELD_MUTATION` ships in the p1 contract surface? Raising
  the frame or lowering the guard are both owner calls; no document states the priority.
- **`STALE_REVISION` raises at two doors inside this slice** (second-pass review, 2026-08-27) —
  the `If-Match` precondition on every mutating verb (`inst-fd-etag`) and the publish pin
  (`inst-fd-publish-pin`) — so §3.3's closed list of two carve-outs is short again, and unlike
  `SCOPE_NOT_CONTAINED` no cross-slice sequencing can retire either arm. Whether the `If-Match`
  precondition counts as one door across all mutating verbs, whether the publish pin is a
  separate door, and whether the rule is per code or per arm are all unstated. The same shape
  reaches `ILLEGAL_FIELD_MUTATION`, which 07 raises on structural-identity attempts.
- **Does a publish bump `internal_revision`?** `inst-fd-transition-guard` says **every**
  transition bumps it and that a re-publish is **not** an edge; the publish transaction
  (`inst-fd-publish-txn`) lists its writes without a bump; and the head-row whitelist admits the
  column "via the save door **and from every transition and correction door**" without naming the
  `PublishDoor`. Three readings follow, and the middle one has teeth: if a re-publish does not
  bump, `published_version` moves while the ETag token does not, so a client's stale cached
  representation still passes its own precondition. Nothing in the tree picks one.
- **A bucket-ii write arriving at the save door: refused, or forwarded?** `inst-fd-mutability-frame`
  says both in one row — the Foundation "routes bucket-ii to slice 07's correction door", and
  bucket-ii writes are "admitted **only through slice 07's correction door**". If it is a refusal
  it has no code: `ILLEGAL_FIELD_MUTATION` is scoped to a bucket-i write, and 07 covers structural
  identity only. Naming a code or specifying route-and-continue is new normative content.
- **A caller-supplied id: codeless 400 or `VALIDATION`?** `inst-fd-mint-id` calls a stray id in the
  payload "a `400`" — the only rule-level HTTP status in the file — while §3.3 reserves the bare
  400 for a malformed request and gives the shape phase its own `VALIDATION` envelope. A stray id
  is a shape-phase finding under §3.1's order, which argues the other way. Unclassified anywhere.
- **Does `products_product` carry `replaced_by_sku_id`?** 04 §4 lists it among "Columns on
  `products_sku`/`products_product` (carried by 01)", and this slice's shared guard is declared
  for both entity tables, but only §4.2 defines the column, on SKUs. A Product pointing at a
  replacement *SKU* is either a naming error in 04 or a real Product-level column; 04's
  retirement-initiation flow states no Product replacement, so adding it here would invent schema.
  *(Its sibling `deprecation_provenance` was the mirror case and is settled: 04 writes provenance
  `direct` on the retiring parent Product, so §4.1 now carries that column.)*
- **On what grounds does the 422 `MUST NOT` stand?** (second-pass cross-document lens, 2026-08-27).
  Its stated reason was false and is now struck: `docs/arch/errors/DESIGN.md` §2.2 permits a
  per-occurrence `TransportOverride` within the same status class, 422 and 400 are both 4xx, and
  the platform's own `extract::Json` renders an `InvalidArgument` at 422. So a canonical 422 *is*
  producible. Neither this design set nor pricing's declares an override anywhere, which is what
  makes the rule true in practice today — but "this gear declines to use overrides" is a decision
  no document records, and pricing states the rule unscoped. No products decision (P-D-01…P-D-20)
  or PRD requirement mentions wire status at all.
- **Is slice 08's convergence probe the owed NFR #3 probe?** This slice says "**The probe is
  owed**: no slice §5 measures it", while 08 C5 already decomposes the budget into
  "commit→durable-acceptance (01's outbox meter) + acceptance→projected (this slice's meter)" and
  its convergence
  probe measures from write commit — but asserts against 08's own budget and expressly refuses to
  collapse the two, "the re-basing C5's M1 fix struck for collapsing budgets NFR #3 keeps
  distinct". Whether one meter may be asserted against two thresholds, or a second probe is owed,
  is settled nowhere; slice 06's open item records the split rather than deciding it.
- **How does a refusal's audit row commit?** (P-D-21, 2026-08-27 — latent before it, load-bearing
  after). §4.4's rule was "every mutating door writes exactly one row in its transaction,
  including every rejection with its reason", and a refusal rolls that transaction back. Under
  P-D-21 refusals are the table's **main** content, so the mechanism can no longer be left
  implicit: an autonomous/second transaction, a door that commits the audit row and rolls back
  only the entity writes, or something else. Each has different failure behaviour when the
  refusal is itself caused by the database being unavailable, and no document in the tree states
  which. **Nothing else in the gear can be relied on to cover it**: the event stream cannot, by
  P-D-21's own reasoning.
- **Where does slice 03's resolved-binding snapshot live now?** `inst-cd-stamp` stamps
  `(gts_id, kind, metadata_fields)` "into the audit row of the publish", and a publish is a
  committed act that under P-D-21 writes no audit row. §15's deletion negotiation and pricing's
  `meter_binding_divergent` remediation are both written to reference it. The publish event
  payload and `products_entity_version` are both plausible homes; the choice is slice 03's and is
  registered here only because P-D-21 was recorded from this slice.
- **The event payload owes the `revision`.** P-D-21 makes the event the success-path audit record,
  and the audit tuple it replaces is `actor_ref`, action, subject `(kind, id, revision)`, reason,
  correlation id. §4.4's stated payload carries the envelope, a versioned schema ref,
  correlation/causation, the idempotency key and `actor_ref`; the event type supplies the action
  and `aggregate_id` the subject id. **`revision` is in neither**, so as it stands the record
  cannot say which revision an act applied to — the one thing an audit trail over a versioned
  entity exists to say. Owed as a payload amendment, not written here.
- **What unit does the one-door rule count — a door, a phase, or an arm?** *(all three lenses of
  the 2026-08-27 third pass raised this independently, on three different codes; it subsumes the
  `SCOPE_NOT_CONTAINED` and `STALE_REVISION` items above.)* §3.3 states the rule over **doors** and
  closes the carve-out list at two, but the identity phase alone raises `DUPLICATE_NAME` and
  `DUPLICATE_SKU_CODE` and runs at three doors (create, save, publish re-run); `VALIDATION` is the
  shape phase's envelope and the pipeline runs "inside every mutating door"; `IDEMPOTENCY_CONFLICT`
  is raised wherever a key resolves; and `CONTENT_PII_BLOCKED` is described three ways across the
  set — this slice calls it doorless, 10 says "the door is 02's", 02 enumerates every door that
  invokes the hook. None of these can be retired by cross-slice sequencing, so the answer is not a
  count but a definition, and it decides whether slice 12's `inst-cc-errors` coverage check is
  writable as stated. Owner: whoever owns §3.3's rule, jointly with slice 12.
- **Which bucket do the Foundation-owned columns sit in?** The physical guard routes by bucket tag,
  but 05's bucket registry records only slice 03's, and the PRD enumerates buckets ii and iii by
  name while leaving i and iv as classes. Unassigned here: `name`/`name_normalized`,
  `region_scope`/`brand_scope`, the parent link, `product_code`, and `cloned_from` — whose §4.2
  note says "immutable after [create]" while the guard's bucket-i clause only bites "never after
  first publish", so if it is bucket-i a rewrite before first publish is permitted and the note is
  wrong. This decides a product-visible question — whether a published Product can be renamed —
  so it is the owner of `fr-field-mutability-matrix`'s call, per column.
- **What refuses a SKU create under a parent that is missing, `retired` or `discarded`?**
  `inst-fd-containment` names a code for its third arm only (`RETIREMENT_PENDING`). 04's
  `PARENT_NOT_PUBLISHED` cannot serve the first two — a `draft` parent is legal at create, so that
  code would refuse legal creates — and 01's own 404 convention is scoped to a path segment, while
  the parent id here is a payload member. Two arms of a `p1` guard terminate in no code and no
  status. Naming one is new normative content; the taxonomy's owner must add a code, or say both
  ride `VALIDATION`.
- **A create-door refusal has no subject id to audit.** §4.4 requires every refusal row to carry
  subject `(kind, id, revision)`, and P-D-21 makes refusals this table's main content — but the
  create flow mints the id *after* the checks that refuse, so `DUPLICATE_NAME`,
  `DUPLICATE_SKU_CODE` and `SCOPE_NOT_CONTAINED` refusals have neither id nor revision. Either the
  mint moves ahead of validation (changing what a rejected create consumes from the id space) or
  the audit tuple must admit an absent subject. Both are owner calls.
- **What happens to a duplicate that arrives while the first request is still in flight?** §3.2
  gives two outcomes, replay and `IDEMPOTENCY_CONFLICT`, and stores `payload_hash · outcome_ref ·
  expires_at` — a shape in which `outcome_ref` cannot exist before the guarded operation produced
  one, so a concurrent duplicate matches neither branch. The donor answers it with machinery this
  gear declares nowhere (a claimed/answered row state, expiry evaluated at claim time rather than
  by this slice's background sweep, and a third code for an in-flight key). Adopting it means a new
  row state and a new code; declining it means concurrent duplicates both execute. Also unstated:
  whether `payload_hash` covers the received bytes or a canonical rendering of the parsed request.
- **Who may select `PreAuthorized(approvalId)`, and what bounds its re-use?** §1.5 puts the mode in
  this slice's scope and §2 says the gate under it consumes nothing, so a caller or a re-run naming
  the same record while the pin still matches publishes again. No document says whether the mode is
  a wire-visible parameter or an internal-only door argument. Interacts with the
  `internal_revision`-on-publish item above: if a re-publish does not bump, the pin never stops
  matching.
- **Where is "engine-canonical serialization" pinned?** §4.3 names it and 06 §3 points back here
  ("the 01 engine-canonical discipline"), so each slice cites the other and neither states the
  rule, while 10's restore drill compares digests byte-for-byte across both engines. §4.1 shows
  what a pin looks like when this file makes one. Someone must fix field order, encoding and the
  null/absent representation — plausibly this slice, since it writes the first digest.
- **Does this slice's transition floor owe events?** §4.5's completeness rule is written as "every
  **other** slice", exempting the one that states it, and this slice's edge list admits three edges
  with no event in its roster. Whether the Foundation's rows must cite 04's
  `SkuDeprecated`/`SkuUndeprecated`/`SkuRetired`, or record "no event" and leave emission to 04, is
  stated nowhere; slice 12 owns the completeness check but not the rule's scope.
- **Which surface serves the columns that only ever exist on the head row?** A deprecation, a
  retirement initiation and a cascade are transitions, not publishes, so `lifecycle_state`,
  `deprecation_provenance` and `replaced_by_sku_id` move on the head row with no new
  `products_entity_version` row to carry them — yet 08 must serve current state and 04 promises a
  transitively resolvable `replacedBy`. §4.3's frozen-only rule and those obligations cannot both
  hold as written. Slice 01 and slice 08 owners jointly: either head rows are a legitimate read
  source for those columns, or a transition writes a version row after all.
- **Is the per-version diff this slice's?** §1.1 claims "history with diff", but §4.3 states only
  the mechanism, §1.5 In has no diff entry, §2 no flow, §3 no instruction, §5 no probe — while 08
  `inst-rh-timeline` is where the diff surface actually lives. Either §1.1 overclaims, or this
  slice owes a door and a probe.
- **Does the registry own `taxCategory`/`glCode` at all?** C3 cites PRD §2.1, and §2.1 puts "billing
  descriptors (invoice line template / tax category / GL code)" among the things "owned elsewhere
  and MUST NOT be re-specified here" — while `fr-accounting-codes` requires the registry to persist
  and validate them and §4.2 carries `tax_category_ref` and `gl_code_ref`. The PRD contradicts
  itself; only its owner can say which sentence governs, and if §2.1 wins this slice loses two
  columns.
