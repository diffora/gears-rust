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
state machine core (`draft → published [↔ deprecated] → retired`, `draft → discarded`,
forward-only), the fail-closed **registered-validator pipeline**, append-only published-version
history with diff, per-row optimistic concurrency (`If-Match` on the internal revision),
tenant-scoped idempotency, the broker-native event fan-out through a transactional outbox
(P-D-01), and the append-only audit trail.

The Foundation deliberately owns **no capability policy**: it does not know what a `PlanTier`,
a metering unit, a materiality threshold, or a freeze participant is. Capability slices author
draft state through Foundation write doors, register their validation rules and their material
field sets, contribute read-model fields, and call the Foundation publish API. Governance
(slice 05) is a **registered gate** inside the publish door, not a separate path around it.

### 1.2 Purpose

Give the eleven capability slices one set of doors with the invariants already paid for:
identity that cannot be reissued, versions that cannot be rewritten, transitions that cannot be
invented, retries that cannot duplicate, events that cannot be lost before they are
acknowledged, and rejections that always carry an audited reason. Per P-D-02, everything
mechanical lives here; every human gate attaches to the entity-publish act.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Authors drafts through the write doors; publishes (via the slice-05 gate) |
| `cpt-cf-bss-products-actor-catalog-admin` | Same doors, wider grants; discard, deferred operations |
| `cpt-cf-bss-products-actor-events-audit` | Receives the outbox fan-out; owns transport (delivery/retry/DLQ) |
| `cpt-cf-bss-products-actor-oss-ams-idp` | Supplies `tenantId`, brand/region claims, roles; the registry never mutates tenant topology |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.1, §6.5, §6.7, §6.13; AC #1, #2 (frame), #5 (uniqueness), #13,
  #14, #27, #28, #38, #42
- [`../DECISIONS.md`](../DECISIONS.md) P-D-01 (envelope), P-D-02 (mechanical increments),
  P-D-04 (absolute name uniqueness)
- Pricing `design/01-foundation.md` — the pattern donor (registered validators, append-only
  triggers with column whitelists, draft/published partial unique indexes, outbox + pending
  refs); divergences are stated where they occur
- `gears/system/event-broker/docs/DESIGN.md` + its ADR-0003 — the envelope this gear emits

### 1.5 Scope

**In**: entity model + storage shape for `Product`/`SKU` core columns; identity + `skuCode`
reservation; revision/version mechanics; state-machine core (edge list, terminality, physical
floor); validation pipeline frame + the error taxonomy; idempotency; ETag concurrency; outbox +
envelope discipline + per-aggregate ordering; audit; the interim parent-child region-containment
check (P-D-04 residue — final rule in slice 04).

**Out** (owned by later slices, listed so absence reads as intent): category/attribute content
rules (02); typing/classification/metering policy (03); deprecation/retirement policy,
scheduling, cascades (04); materiality, approvals, RBAC grants, break-glass (05);
`CatalogVersion` and freezes (06); `SkuReferenceCount` and corrections (07); read models (08);
bulk (09); retention/erasure execution (10); clone (11); seam suite + replay/bootstrap (12).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Dual-engine storage (SQLite + Postgres); one migration per table, guards defined once; schema oracle goldens from day one | house practice (pricing migration-chain rebuild) |
| C2 | Broker-native envelope; no CloudEvents field anywhere in the payload path | P-D-01 |
| C3 | No money, no price, no charge computation in this gear | PRD §2.1 |
| C4 | Every table carries `tenant_id`; all repository access through SecureORM tenant scoping | PRD §6.8; ToolKit |
| C5 | Append-only posture: published rows and history rows are physically guarded (REVOKE + trigger whitelist), not just conventionally. Exempt by design: the slice-08 projection family (rebuildable state, not records) and expiring operational stores (idempotency sweep) | PRD `fr-revision-vs-version` |
| C6 | Idempotency-key retention ≥ 24h **and** ≥ the maximum freeze timeout (slice 06 exports the number; the store reads it as config) | PRD AC #27 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `RegistryEntity` | The trait both `Product` and `SKU` implement toward the pipeline: kind, id, tenant, lifecycle state, internal revision, published version |
| `ValidationPipeline` | The ordered, fail-closed run of shape → identity → registered slice validators → governance gate, executed inside every mutating door |
| `RegisteredValidator` | A slice-contributed rule keyed by `(entity kind, transition or field set)`; registration is code, not config |
| `PublishDoor` | The single Foundation API that turns an approved draft into a published version (bump, snapshot, events) — the only writer of `published_version` |
| `ReservationIndex` | The partial unique index realizing `skuCode` reservation (see §4.2) |
| `identity-reference map` | The pseudonym → operator identity table audit/events point at; its erasure semantics are slice 10's |

### 1.8 Context & Dependencies

**Consumed**: IdP claims (tenant/brand/region/roles); event-broker SDK (publish, durable ack);
platform config store (interim policy defaults, PRD §17.1). **Produced**: Foundation events
(§4.5), audit rows, the SDK read/write surface the studio and sibling gears call. **Explicitly
not consumed here**: `SkuReferenceCount` (slice 07), pricing signals (06/07).

## 2. Actor Flows (CDSL)

### Create a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-create-product`

1. [ ] - `p1` - Authorize `product × write` in the caller's tenant/brand scope (deny-by-default); resolve the idempotency key `(tenant, endpoint, client key)` — a hit with identical payload replays the stored outcome, a hit with different payload fails `IDEMPOTENCY_CONFLICT` - `inst-fd-idempotency`
2. [ ] - `p1` - Validate shape; normalize `name`; enforce **absolute** uniqueness on `(tenant_id, brand_id, name_normalized)` via the partial unique index (§4.1) — collision fails `DUPLICATE_NAME` naming the holder; P-D-04: region scope plays no part - `inst-fd-name-unique`
3. [ ] - `p1` - Mint `productId` (UUID, server-side, never caller-supplied — a stray id in the payload is a `400`); optional `productCode` reserves under the same rules as `skuCode` - `inst-fd-mint-id`
4. [ ] - `p1` - Persist as `draft`, `published_version = 0`, `internal_revision = 1`; write the audit row and the `ProductCreated` outbox row in the same transaction - `inst-fd-create-txn`

### Define a SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-define-sku`

1. [ ] - `p1` - Authorize; idempotency as above - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - Parent must exist in the tenant, not be `retired`/`discarded`, **and not hold a live retire intent** (`RETIREMENT_PENDING` — item 36 of the 2026-08-26 review: a `deprecated` parent still admits children, so a draft SKU created after the `CascadePlan` was computed is outside the plan's auto-discard arm and defers that Product's retirement indefinitely); the SKU's brand/region scope must pass the **interim containment check**: scope sets are flat value lists, containment = subset, anything not provably a subset fails `SCOPE_NOT_CONTAINED` (conservative until slice 04 pins the final rule) - `inst-fd-containment`
3. [ ] - `p1` - Reserve `skuCode` **atomically at create**: the insert itself is the reservation — the `ReservationIndex` admits exactly one non-`discarded` holder per `(tenant_id, sku_code)`; the loser of a concurrent race fails `DUPLICATE_SKU_CODE` with an audited reason (PRD AC #42) - `inst-fd-reserve-code`
4. [ ] - `p1` - Mint `skuId`; persist as `draft` with the slice-03-owned columns present but unjudged (typing/classification rules run when slice 03 registers them); audit + `SkuCreated` outbox row in the same transaction - `(cont. inst-fd-create-txn)`

### Save an edit (draft or published head)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-save-draft`

1. [ ] - `p1` - Every mutating verb on an entity head **requires `If-Match`** on the internal revision; mismatch fails `STALE_REVISION`; an absent precondition is a malformed request (per-row token, never plan-shared — the pricing D-141 lesson adopted at birth) - `inst-fd-etag`
2. [ ] - `p1` - Run the pipeline's shape + identity phases plus every registered validator for `(kind, field set)`; violations collect per-field into one audited rejection - `inst-fd-pipeline`
3. [ ] - `p1` - Saves land on the **head row** — the authoring surface for `draft`, `published`, and `deprecated` entities alike (H1 fix, 2026-08-25 review): a save is never a lifecycle transition, and consumers are untouched because **every consumer-facing read serves frozen `products_entity_version` content, never the head row**. A `skuCode` change is legal only while `published_version = 0` and releases the old code by the row update itself; `internal_revision += 1`; audit + outbox in the same transaction. Saves **never** touch `published_version` - `inst-fd-save-txn`
4. [ ] - `p1` - A draft save on an entity holding an open approval **invalidates it** — the Foundation raises the `approval-invalidated` hook; slice 05 owns re-queue semantics - `inst-fd-approval-hook`

### Discard a never-published draft

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-discard`

1. [ ] - `p1` - Legal only from `draft` with `published_version = 0`; transition to `discarded` (terminal); the `ReservationIndex` excludes `discarded` rows, so the `skuCode`/`productCode` reservation releases by the same write; audit + `SkuDiscarded`/`ProductDiscarded` event - `inst-fd-discard`

### Publish an entity (the mechanics half)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-publish`

1. [ ] - `p1` - `PublishDoor` accepts `(entity, expected internal revision)` — a `draft` for its first publish, or a `published`/`deprecated` **head** for version N+1 (a re-publish changes the version, never the state); stale revision fails `STALE_REVISION` — an approval is only usable against the exact revision it pinned (slice 05 stores the snapshot; the Foundation enforces the match) - `inst-fd-publish-pin`
2. [ ] - `p1` - Re-run the **full** pipeline at publish (shape, identity, every registered validator for the `→ published` transition): an entity that stopped being publishable since approval fails closed `INCOMPLETE_ENTITY`/rule-named code, never publishes stale - `inst-fd-publish-revalidate`
3. [ ] - `p1` - The governance gate (slice 05) runs **inside** the door, and the door therefore carries an explicit **authorization mode** (Blocking 9 fix, 2026-08-26 review): `Gate` — the ordinary interactive publish, which needs a `satisfied` record — or **`PreAuthorized(approvalId)`**, the mechanical stage of a composite act (05 `inst-gv-one-shot`: scheduled activation, a cascade leg, a bulk row, the composition clear). Under `PreAuthorized` the gate does not look for a `satisfied` record and does not consume one; it **verifies** that the named record authorized *this* subject and that its pinned revision still matches, raising `APPROVAL_REQUIRED` only when it did not. Without the mode the two readings collide and every scheduled publish fails terminally: the runner drives "the ordinary publish door" (04 `inst-sp-activate`), the gate inside it would see a `consumed` record, and 04 `inst-ar-failure` wraps that into a terminal `SCHEDULE_STALE_APPROVAL`. Re-validation stays fail-closed in both modes — the mode governs *who approved*, never *whether the entity is still publishable*. A material change without satisfied approvals fails `APPROVAL_REQUIRED`; the Foundation knows only "the gate answered yes/no + reason". An approval rejection "returns the entity to draft" (AC #26) reads: a first-publish entity stays `draft`; a published head keeps its pending edits unpublished — no state flip either way (design reading under the head-row model, **flagged**: the literal reading would need a `published→draft` edge the PRD's own forward-only rule forbids — slice-05 review L-6) - `inst-fd-governance-gate`
4. [ ] - `p1` - On yes: `published_version += 1` (the door is this column's **only** writer); freeze the full entity content into `products_entity_version`; first publish makes `skuCode` reservation permanent (immutability enforced by the trigger whitelist from this row-state on); emit `ProductPublished`/`SkuPublished`; audit — one transaction - `inst-fd-publish-txn`
5. [ ] - `p1` - Post-commit, slice 06 consumes the publish event **as content only** (what became publishable); an entity publish **never enqueues a CatalogVersion increment** — addressability comes from downstream requests or an operator catalog-publish act (06 `inst-cv-request`; M1 fix of the 06 review), and the Foundation itself requests nothing (P-D-02) - `inst-fd-publish-fanout`

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
4. [ ] - `p2` - Field-mutability enforcement frame: each published-state field carries a bucket tag (i structural / ii correctable / iii material-mutable / iv descriptive — PRD `fr-field-mutability-matrix`); the Foundation refuses bucket-i writes outright and routes bucket-ii to slice 07's correction door; bucket iii/iv are ordinary head-row saves re-published as version N+1, their materiality judged by slice 05; bucket-ii writes are admitted **only through slice 07's correction door**, which the physical guard (§4.2) knows by name - `inst-fd-mutability-frame`

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
same code, so this is the one code in the gear with two raising doors, and §3.3's
one-door rule is stated per **arm** for it). Every code appears in exactly one raising **door** — a door, not a slice: validators registered into the `PublishDoor` raise through it;
slice-owned codes (taxonomy cycles, unit rules, freeze, bulk rows…) are declared in their
slices and the AC #38 ↔ code ↔ slice map is completed by slice 12's coverage check. Codes are
part of the SDK contract; renames are breaking.

### 3.4 Concurrency doors (PRD §6.13 residents of this slice)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-concurrency`

1. [ ] - `p1` - `skuCode` race: decided by the `ReservationIndex` under the insert, not by a read (two concurrent creates: exactly one admitted) - `inst-fd-code-race`
2. [ ] - `p1` - Draft race: decided by `If-Match` (two editors: second fails `STALE_REVISION`) - `(cont. inst-fd-etag)`
3. [ ] - `p1` - Publish-vs-edit race: the door's pinned-revision check makes "approve rev N, publish rev N+1" impossible by construction - `(cont. inst-fd-publish-pin)`

## 4. Data / Storage (normative shape; DDL in migrations)

### 4.1 `products_product`

`product_id` (PK, uuid) · `tenant_id` · `brand_id` · `name` · `name_normalized` ·
`product_code` (nullable) · `lifecycle_state` (`draft|published|deprecated|retired|discarded`) ·
`internal_revision` · `published_version` · **category assignments live ONLY in slice 02's `products_product_category`** (the assignment
table with the exactly-one-primary partial index — a second inline representation here would
be a divergence channel with no authority rule; the frozen version content carries the
assignment set as a copy at publish, like every other content class) · `region_scope` /
`brand_scope` · `created_by` (pseudonymous ref) · `cloned_from` (create-only, immutable —
slice 11) · timestamps.

Indexes/guards: **partial UNIQUE `(tenant_id, brand_id, name_normalized) WHERE lifecycle_state
<> 'discarded'`** (P-D-04; discard releases the name exactly as it releases codes — a
design-introduced symmetry, flagged in §6); partial UNIQUE on `(tenant_id, product_code) WHERE
product_code IS NOT NULL AND lifecycle_state <> 'discarded'`; append-only trigger enforcing the
shared head-row guard (§4.2).

### 4.2 `products_sku`

`sku_id` (PK, uuid) · `tenant_id` · `product_id` (FK) · `sku_code` · `type`
(`product|service|bundle`) · `lifecycle_state` · `deprecation_provenance`
(nullable `direct|cascaded`, slice 04) · `sellable` (default `true`, D-46) · `plan_tier` ·
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
from slice 04's own doors** (deprecation/cascade and retirement-initiation respectively — they
are neither save-door nor bucket-iii/iv columns, and leaving them unnamed either refused the
writes slice 04 specifies or dropped them to bucket iv, where an ordinary operator save could
re-stamp the provenance operand `inst-lc-provenance-reversal` reads — item 18 of the 2026-08-26
review); bucket-ii columns only via the slice-07 correction door; bucket-i identity columns
never after first publish. **`normalized(name)`** (the uniqueness and promotion-identity operand,
P-D-04/AC #33a) is pinned: Unicode NFKC → full casefold → trim + collapse internal whitespace
to single spaces, computed **application-side** so both engines store identical bytes.

### 4.3 `products_entity_version` (published history)

`(tenant_id, entity_kind, entity_id, published_version)` UNIQUE · frozen full content **excluding the metadata map** (P-D-06 — the map lives beside the entity,
captured only by `CatalogVersion` snapshots) · **a per-row content digest written at freeze**
(the slice-10 restore drill re-verifies sampled entity versions against it — without it,
version-history corruption is invisible to every checksum; H2 fix) — the
publish-time entity, engine-canonical serialization — the byte-identity discipline that
`CatalogVersion` (slice 06) will reuse) · `approval_ref` · `actor_ref` · `published_at`.
Append-only, no UPDATE path at all; diffs are computed between rows, never stored mutated.
These rows are the **only consumer-read surface** for entity content: read models,
`CatalogVersion`, and the SDK project from here — never from head rows.

### 4.4 `products_idempotency`, `products_audit_log`, `products_outbox`

- `products_idempotency`: `(tenant_id, endpoint, client_key)` PK · `payload_hash` ·
  `outcome_ref` · `expires_at` (§3.2).
- `products_audit_log`: append-only; `actor_ref` (pseudonymous — the identity-reference map is
  slice 10's), action, subject `(kind, id, revision)`, reason, correlation id. Every mutating
  door writes exactly one row in its transaction, including every rejection with its reason.
  **Reserved platform-sealing seam (P-D-08)** — present from the first migration, never written
  by this gear: `seal_state` (NOT NULL, roster `unsealed | sealed`, written **`unsealed` at
  INSERT** by this gear — always, in v1 and after activation alike — which makes the unproven
  era queryable instead of inferred from a deployment date; **the trigger whitelist admits
  exactly one UPDATE on this column group: a one-way `unsealed → sealed` transition supplying
  `chain_id`/`seq`/`row_hash` in the same statement, under the platform sealer's own identity,
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
  sequence)` monotonic per aggregate; dispatcher publishes to the event-broker and marks
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

**Traces to**: `cpt-cf-bss-products-usecase-product-sku-editor` (§10 use case, claimed by id here 2026-08-26 — all seven were in lint 1's universe and none was claimed); **NFRs by id** — #6 `cpt-cf-bss-products-nfr-scale-extensibility` (the entity-count half: the head/version split and the index shape; `CatalogVersion` growth is slice 06's), #8
`cpt-cf-bss-products-nfr-determinism-integrity` (version immutability, taxonomy acyclicity, identity uniqueness and
metering-unit validity enforced fail-closed: this slice's pipeline, edge list and trigger
whitelist are its whole mechanism, and it was referenced nowhere in the set — item 30 of the
2026-08-26 review); **§9 by id** — `cpt-cf-bss-products-interface-authoring-publish` (§9.1: this slice owns the authoring and publish doors, idempotency keys, `If-Match`, intent semantics) and `cpt-cf-bss-products-contract-registry-events` (§9.2 outbound: the broker-native envelope + outbox fan-out are this slice's). *(§9 ids were claimed by no slice's Traces-to at all until the 2026-08-26 branch review — prose like "§9 (all seven id-bearing blocks)" is not the id lint 1 keys on, so it would have reported zero claims for the whole §9 surface on its first run. Exactly the item-30 defect, left standing for §9 by the wave that widened the lint to include it.)* `cpt-cf-bss-products-fr-identifier-contract`, `cpt-cf-bss-products-fr-create-product` (uniqueness carrier),
`cpt-cf-bss-products-fr-define-sku` (identity carrier), `cpt-cf-bss-products-fr-revision-vs-version` (counters + history halves; the
version-binding-at-freeze clause → slice 06), `cpt-cf-bss-products-fr-lifecycle-transitions`
(machine core), `cpt-cf-bss-products-fr-idempotent-authoring`, `cpt-cf-bss-products-fr-registry-eventing-audit` (envelope + outbox
half), `cpt-cf-bss-products-fr-event-delivery-resilience` (registry-side half: durable acceptance + outbox),
`cpt-cf-bss-products-fr-parent-child-integrity` (interim containment half; final rule → slice 04),
`cpt-cf-bss-products-fr-skucode-reservation-concurrency`, `cpt-cf-bss-products-fr-field-mutability-matrix` (enforcement frame),
`cpt-cf-bss-products-fr-expected-failure-behavior` (taxonomy home); AC #1, #5 (name-uniqueness half), #13, #14, #27, #28 (envelope), #38
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
