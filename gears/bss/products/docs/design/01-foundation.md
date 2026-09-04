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
  - [Save an edit (draft, published or deprecated head)](#save-an-edit-draft-published-or-deprecated-head)
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
  - [4.4 `products_idempotency`, `products_audit_log`, the toolkit outbox](#44-products_idempotency-products_audit_log-the-toolkit-outbox)
  - [4.5 Foundation-owned events](#45-foundation-owned-events)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

The Foundation is the shared engine every products-gear capability publishes through: the
`Product`/`SKU` entity model and identity rules (server-minted UUIDs, atomically reserved
`skuCode`), the two version counters (internal revision vs published version), the lifecycle
state machine core (`draft → published ↔ deprecated → retired`, `draft → discarded`;
no edge back to `draft`), the fail-closed **registered-validator pipeline**, append-only published-version
history (the *diff over* it is slice 08's `inst-rh-timeline`, and the catalog-version diff is
06's — this slice owns the frozen entity rows both read — not all of their input, since
`fr-catalog-version-diff` also covers the metadata map §4.3 excludes, and not a diff surface of
its own; this section had
claimed it until the owner's call of 2026-08-27), per-row optimistic concurrency (`If-Match` on the internal revision),
tenant-scoped idempotency, the broker-native event fan-out through the toolkit outbox
(P-D-01 envelope, P-D-22 outbox), and the append-only audit trail — which under **P-D-21** records only what emits no
event: refusals, reads under elevation, and committed acts the design declares emit no broker
event.

The Foundation deliberately owns **no capability policy**: it does not know what a `PlanTier`,
a metering unit, a materiality threshold, or a freeze participant is. Capability slices author
draft state through Foundation write doors, register their validation rules and their material
field sets, contribute read-model fields, and call the Foundation publish API. Governance
(slice 05) is a **registered gate phase** inside the pipeline, hosting any gated act — publish or
transition alike (P-D-30) — not a separate path around it.

### 1.2 Purpose

Give the eleven capability slices one set of doors with the invariants already paid for:
identity that cannot be reissued, versions that cannot be rewritten, transitions that cannot be
invented, retries that cannot duplicate, events that cannot be lost before they are
acknowledged, and rejections that always carry an audited reason. Per P-D-02, every governance gate attaches to the **entity publish that introduces the exception** — and per P-D-30 the gate *phase* hosts any gated act, publish or transition alike (§3.1).

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Authors drafts through the write doors; publishes (via the slice-05 gate) |
| `cpt-cf-bss-products-actor-catalog-admin` | Same doors, wider grants; discard, deferred operations |
| `cpt-cf-bss-products-actor-events-audit` | Receives the outbox fan-out; owns transport (delivery/retry/DLQ) |
| `cpt-cf-bss-products-actor-oss-ams-idp` | Supplies `tenantId`, brand/region claims, roles; the registry never mutates tenant topology |

### 1.4 References

- [`../PRD.md`](../PRD.md) §2.1, §6.1, §6.5, §6.7, §6.8, §6.13, §9, §10, §15, §17.1; AC #1, #2 (frame), #5 (uniqueness), #13,
  #14, #26, #27, #28, #33a, #38, #42
- [`../DECISIONS.md`](../DECISIONS.md) P-D-01 (envelope), P-D-02 (mechanical increments),
  P-D-04 (absolute name uniqueness), P-D-06 (metadata-map placement), P-D-08 (audit-sealing
  seam), P-D-14 (`system_signal` subjects), P-D-21 (the audit table holds only what emits no event), P-D-22 (the outbox is the toolkit's),
  P-D-23 (the owner round, recorded inline in the rules they change; the register
  carries the authoritative table), and the same day's later
  rounds, each cited in the rule it changes: P-D-24 (the `state` phase), P-D-25 (the completed
  error contract), P-D-26 (four transaction boundaries), P-D-27 (the event contract), P-D-28
  (four read paths the guard needed), P-D-29 (what a replay, an envelope and a digest carry),
  P-D-30 (gate host, authorization, whose validator), P-D-31 (the four routed outward, decided
  here), P-D-32 (the second lens wave's six calls), P-D-33 (eight calls from weeding the open items),
  P-D-20 (the retirement lead window imposes no publish freeze), P-D-34 (the remaining items, decided from the set), P-D-35 (the five the set already forced), P-D-36 (the phase unit withdrawn), P-D-37 (one code per row, all violations in the answer), P-D-38 (a refusal stores nothing), P-D-39 (scope columns and the empty set), P-D-40 (the version table's one admitted DELETE), P-D-41 (the two bucket-ii doors), P-D-42 (the store's last three operands), P-D-43 (the checking layer's four grammars), P-D-44 (the AC #38 map), P-D-45 (the last four lint grammars), P-D-46 (four write-path blockers), P-D-47 (the last four blockers: a tombstone state, a withdrawn opt-in, two codes, the broker's producer), P-D-48 (the six flagged decisions put to the owner; this slice gains the re-announcement row), P-D-49 (six live contradictions; here the takeover CAS and the successor column's second admitted write), P-D-50 (the pre-implementation round; here the `BucketRegistry` miss is fail-closed and §5's agreement test gains its third assertion)
- Pricing `design/01-foundation.md` — the pattern donor (registered validators, append-only
  triggers with column whitelists, draft/published partial unique indexes, pending refs — **not
  the outbox**, which P-D-22 moved to `toolkit_db::outbox` after measuring that pricing runs a
  private `pricing_outbox` of its own); divergences are stated where they occur
- `gears/system/event-broker/docs/DESIGN.md` + its ADR-0002 (partition selection), ADR-0003 (the envelope this gear emits) and ADR-0004 (producer modes); `gears/system/event-broker/event-broker-sdk` — the outbox-backed producer this gear publishes through (**P-D-47**)

### 1.5 Scope

**In**:
- entity model + storage shape for `Product`/`SKU` core columns
- identity + `skuCode`/`productCode` reservation
- revision/version mechanics
- state-machine core (edge list, terminality, physical floor)
- validation pipeline frame + the error taxonomy
- the gear's five wire doors — create, save, publish, discard and the authoring head read (§2,
  P-D-31/P-D-33) — and the governance gate's host contract (`Gate` / `PreAuthorized(approvalId)`);
  the grants they spend are 05's
- field-mutability enforcement frame (bucket routing)
- idempotency
- ETag concurrency
- the toolkit outbox's enqueue path (P-D-22), the broker SDK's producer on top of it, envelope
  discipline, and per-tenant ordering by the broker's partition selection (P-D-47)
- audit of the acts that emit no event (refusals; reads under elevation; committed acts declared
  to emit no broker event — P-D-21)
- the interim parent-child brand/region containment check (P-D-04 residue — final rule in slice 04)
- resolution and first-appearance minting of the acting principal's `actor_ref` through slice 10's
  `products_identity_ref` (`inst-fd-actor-ref`)
- the reserved platform audit-sealing seam's columns, CHECK and one-way trigger (P-D-08 — present
  from the first migration, never sealed here; §4.4)
- `cloned_from`'s create-only immutability, the `PublishDoor`'s `composition_pending` write, the
  interim row-image predicates on `deprecation_provenance` and `replaced_by_sku_id` (P-D-34, until
  04 supplies tighter ones), `products_audit_log`'s retention DELETE arm, and `products_entity_version`'s one admitted
  DELETE under its referential predicate (P-D-40, §4.3; the GC act behind it is 10's, and it
  costs 06 an index) — the columns'
  **guards**, which ride this slice's first migration and publish door; the clone semantics behind
  one are 11's, the composition semantics behind another 06's, the deprecation and retirement
  semantics behind two 04's, and the retention window behind the arm 10's.

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
| C5 | Append-only posture: head rows, history rows and `products_audit_log` are physically guarded (the trigger whitelist on **both** engines, and **P-D-46**: it is the *whole* guard on both — the `REVOKE` arm **P-D-35** had made Postgres-only is withdrawn, the donor gear declining it in both engine tiers for a reason that holds here verbatim — "it names a deployment role the migration does not own" (`gears/bss/pricing/pricing/tests/postgres_approval.rs`, and the SQLite twin); the same way **P-D-31** kept the guard row-image rather than door-reading), not just conventionally. Exempt by design: the slice-08 projection family (rebuildable state, not records) and expiring operational stores (idempotency sweep) | PRD `fr-revision-vs-version`; `fr-registry-eventing-audit` (the audit-log arm) |
| C6 | Idempotency-key retention ≥ 24h **and** ≥ the maximum freeze timeout (slice 06 exports the number; the store reads it as config) | PRD AC #27 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `RegistryEntity` | The trait both `Product` and `SKU` implement toward the pipeline: kind, id, tenant, lifecycle state, internal revision, published version |
| `ValidationPipeline` | The ordered, fail-closed run of idempotency resolution → precondition → shape → state → identity → registered validators → governance gate (any gated act, §3.1), executed inside every mutating door |
| `RegisteredValidator` | A slice-contributed rule keyed by `(entity kind, transition, target state, or field set)` (the target-state variant added by **P-D-32**, which the publish re-run needs); registration is code, not config |
| `BucketRegistry` | The Foundation-owned map from a published-state column to its bucket tag (i–iv), which the head **door** reads — in the application layer — to route a write (§3.1). A slice registers its own columns' tags exactly as it registers validators — code, not config. **The registry is advisory for the physical layer** (**P-D-32**): a compile-time Rust map has no read path from a migration-time trigger, so §4.2's column classes stay static DDL — generating them would break C1's "guards defined once" and the schema-oracle goldens — and §5 carries the test that asserts the two agree. 05 reads the same registry to judge materiality (owner's call, 2026-08-27, P-D-28: 05 already attributes the frame here, and a physical guard of the Foundation's cannot depend on a capability slice's artifact). **A lookup miss is fail-closed** (**P-D-50**): the registry is compile-time, so a published-state column carrying no tag means it was added without registering one, and the head door refuses the write under the pipeline's own posture rather than routing to a default bucket |
| `PublishDoor` | The single Foundation API that turns an approved draft into a published version (bump, snapshot, events) — the only writer of `published_version` |
| `ReservationIndex` | The partial unique index realizing `skuCode` reservation (see §4.2) |
| `identity-reference map` | The pseudonym → operator identity table audit/events point at; its erasure semantics are slice 10's |

### 1.8 Context & Dependencies

**Consumed**: IdP claims (tenant/brand/region/roles); event-broker SDK (publish, durable ack);
platform config store (interim policy defaults, PRD §17.1); slice 10's `products_identity_ref`
(resolve + mint, `inst-fd-actor-ref`). **Produced**: Foundation events
(§4.5), which under P-D-21 are the success-path audit record; refusal, elevation and declared-no-event audit rows;
the SDK read/write surface the studio and sibling gears call. **Explicitly
not consumed here**: `SkuReferenceCount` (slice 07), pricing signals (06/07).

## 2. Actor Flows (CDSL)

*Doors (owner's call, 2026-08-27, P-D-31 — the gear's primary surface had no wire shape, and
12's door×grant lint had nothing to match; paths follow the set's established form):*
`POST /bss-products/v1/products` and `POST /bss-products/v1/skus` (**`product|sku × write`**) →
**201**; `PATCH /bss-products/v1/{products|skus}/{id}` (same grant, **`If-Match` required**) →
**200**; `GET /bss-products/v1/{products|skus}/{id}` (**`… × read`**) → **200** with the head's
internal revision as its **`ETag`** — the authoring read `inst-fd-etag`'s precondition depends on
(**P-D-33**: no surface returned it, so an author who had not just written could obtain no
precondition at all); `POST /bss-products/v1/{products|skus}/{id}/publish`
(**`… × publish`**, **`If-Match` required** — P-D-33 makes the door's pinned revision arrive the
same way every other head verb's does, rather than as an unnamed argument) → **200**;
`POST /bss-products/v1/{products|skus}/{id}/discard` (**`… × discard`**, **`If-Match` required**) → **200**. Every
mutating door accepts **`Idempotency-Key`** (§3.2) — *accepts*, not requires: the PRD scopes the
guarantee to "a retried create/update/publish **with an idempotency key**", so a keyless request
runs with no claim row and no replay, and the resolution phase is skipped rather than failing
(**P-D-34**), and every one of them is an ordinary `Gate`-mode
caller — `PreAuthorized` is never wire-visible (§2 publish). The transition floor has no wire door
of its own: `draft→published` and `draft→discarded` are the publish and discard doors above, and
its **three remaining** edges are driven by slice 04's surfaces.

### Create a Product

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-create-product`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Resolve the acting principal to its `actor_ref` through slice 10's
`products_identity_ref` — 10 `inst-im-map` states the obligation from its side ("01's doors mint
refs through it") and this slice never carried it, while `created_by`, the frozen version's
`actor_ref`, the audit row's and the envelope's all store the result - `inst-fd-actor-ref`
   - [ ] - `p1` - Runs **in its own transaction, before the authorization gate and any phase that can refuse** (owner's call,
     2026-08-27, P-D-26: a refusal rolls the door's transaction back while the refusal's audit row
     commits independently and requires an `actor_ref`, so a first-time principal whose opening act
     is refused would otherwise have no ref to attribute it to) - `inst-fd-actor-ref-txn`
   - [ ] - `p1` - **Mints on a principal's first appearance with no live ref** (10 `inst-im-map`: a
     tombstoned ref is retired permanently, so a principal acting after its erasure mints a fresh
     one); a ref is a pseudonym rather than a domain record, so minting one for a principal that
     was then refused costs nothing; **no event** - `inst-fd-actor-ref-mint`
   - [ ] - `p1` - Resolving advances `last_seen_at`, which 10's age-based erasure reads;
     **no event** - `inst-fd-actor-ref-seen`
2. [ ] - `p1` - Authorize `product × write` in the caller's tenant/brand scope (deny-by-default); resolve the idempotency key `(tenant, endpoint, client key)` — a hit with identical payload replays the stored outcome, a hit with different payload fails `IDEMPOTENCY_CONFLICT`, and a matching-payload hit on a `claimed`, unanswered key fails `IDEMPOTENCY_KEY_IN_FLIGHT` (§3.2 `inst-fd-idem-claim`) - `inst-fd-idempotency`
3. [ ] - `p1` - Validate shape; normalize `name`; enforce **absolute** uniqueness on `(tenant_id, brand_id, name_normalized)` via the partial unique index (§4.1) — collision fails `DUPLICATE_NAME` naming the holder; P-D-04: region scope plays no part; this door's shape, identity and mint steps run in the phases §3.1 assigns them, not in the order numbered here - `inst-fd-name-unique`
4. [ ] - `p1` - `brand_id` is a **required payload field, validated against the caller's brand claims** and refused `VALIDATION` when it names a brand the caller does not hold (**P-D-33**: it is an operand of §4.1's uniqueness index and bucket-i, and deriving it silently from claims breaks on a principal holding more than one brand). Mint `productId` (UUID, server-side, never caller-supplied — a stray id in the payload is a shape-phase
finding and rides `VALIDATION`, owner's call 2026-08-27: the request parsed, so the bare 400 this
gear reserves for a malformed request does not apply, and this had been the file's only rule-level
status with no code); optional `productCode` reserves under the same rules as `skuCode` - `inst-fd-mint-id`
5. [ ] - `p1` - `region_scope`/`brand_scope` are **optional payload value sets** written by this door, **absent meaning the empty set and the empty set meaning unrestricted** (**P-D-39**; PRD §10's operator flow already puts brand/region scope on this surface, and nothing wrote it). Unlike `brand_id` they are **not** validated against the caller's claims: they say where the Product may be sold, not who owns it - `inst-fd-scope-write`
6. [ ] - `p1` - Persist as `draft`, `published_version = 0`, `internal_revision = 1`; write the `ProductCreated` outbox row in the same transaction (**P-D-21**: the event is the
success-path audit record; no audit row is written on a committed act) - `inst-fd-create-txn`

### Define a SKU

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-define-sku`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `actor_ref` resolution, authorize, and idempotency as at create - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - The create door's parent and scope guards, in the phases §3.1 assigns them - `inst-fd-containment`
   - [ ] - `p1` - **Parent must exist in the tenant** — an unresolvable `productId` in the payload
     is a reference the door cannot process, so it rides `VALIDATION` (owner's call,
     2026-08-27) - `inst-fd-containment-parent-exists`
   - [ ] - `p1` - **Parent must not be `retired`/`discarded`** — a refusal by the parent's current
     state rather than by the payload, so **`PARENT_TERMINAL`** (409, same call); the *name* is
     this wave's and the taxonomy owner's to veto, the split is the
     decision - `inst-fd-containment-parent-state`
   - [ ] - `p1` - **Parent must not hold a live retire intent** (`RETIREMENT_PENDING`) — **the
     operand is read by a slice-04 validator registered on this door, not by the Foundation**
     (owner's call 2026-08-27, confirmed against 04's contrary reading by P-D-30), keeping the
     floor policy-free as §1.1 states and leaving `products_scheduled_transition` and `CascadePlan`
     wholly 04's. Item 36 of the review: a `deprecated` parent still admits children, so
     a draft SKU created after the `CascadePlan` was computed is outside the plan's auto-discard
     arm and defers that Product's retirement
     indefinitely - `inst-fd-containment-retire-intent`
   - [ ] - `p1` - **The SKU's brand/region scope must pass the interim containment check**: scope
     sets are flat value lists, containment = subset, anything not provably a subset fails
     `SCOPE_NOT_CONTAINED` (conservative until slice 04 pins the final rule). **Containment is
     defined over restrictions, not over raw sets** (**P-D-39**), which the empty-set reading
     forces: an **unrestricted parent contains every child**, and an **unrestricted child is
     contained only by an unrestricted parent** — a child that sells everywhere cannot sit under a
     parent that sells in three regions. Between two non-empty sets it is ordinary subset. **A SKU
     whose payload omits either set takes the parent's**, so an inherited scope is contained by
     construction - `inst-fd-containment-scope`
3. [ ] - `p1` - Reserve `skuCode` **atomically at create**: the insert itself is the reservation — the `ReservationIndex` admits exactly one non-`discarded` holder per `(tenant_id, sku_code)`; the loser of a concurrent race fails `DUPLICATE_CODE` with an audited reason (PRD AC #42) — **one code covers both reservations**, `skuCode` and `productCode` alike (owner's call, 2026-08-27, P-D-25: `productCode` reserves "under the same rules", so one rule carries one code; the SKU-named form it replaces was declared before `productCode` had an index) - `inst-fd-reserve-code`
4. [ ] - `p1` - Mint `skuId`; persist as `draft` with the slice-03-owned columns present but unjudged (typing/classification rules run when slice 03 registers them); emits the `SkuCreated` outbox row in the same transaction - `(cont. inst-fd-create-txn)`

### Save an edit (draft, published or deprecated head)

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-save-draft`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `actor_ref` resolution, authorize, and idempotency as at create - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - Every mutating verb on an entity head — **the publish door included**, whose pin is that same
header since P-D-33 (`inst-fd-publish-pin`) — **requires `If-Match`** on the internal revision; mismatch fails `STALE_REVISION`; an **absent** precondition rides `VALIDATION` (P-D-33: the
request parsed, so `inst-fd-mint-id`'s criterion applies and the bare 400 this gear reserves for a
malformed request does not) (per-row token, never plan-shared — the pricing D-141 lesson adopted at birth) - `inst-fd-etag`
3. [ ] - `p1` - Run the pipeline's shape + state + identity phases plus every registered validator for `(kind, field set)`; violations collect per-field into one audited rejection - `inst-fd-pipeline`
4. [ ] - `p1` - Saves land on the **head row** — the authoring surface for `draft`, `published`, and `deprecated` entities alike (H1 fix): a save is never a lifecycle transition, and consumers are untouched because **every consumer-facing read of Product/SKU entity content serves frozen `products_entity_version` content, never the head row**. A **bucket-i** change — `skuCode`/`productCode`, `brand_id`, and a SKU's parent `product_id` (P-D-33: §4.2 admits the class on an unpublished head and this is its only admitting door) — is legal only while `published_version = 0` and releases the old code by the row update itself; **a bucket-ii change is admitted here on the same terms** (**P-D-41**: §4.2 admits the class while `published_version = 0`, 03 `inst-mt-bucket` says the draft plane edits freely, and P-D-28's test — an admitted class needs a named admitting door — had left this one unnamed; after first publish it is 07's correction act alone); `internal_revision += 1`; **the entity's content rows in the slices' own tables — 02's category assignments and attribute values, 03's metering declaration — written by this door in this transaction** (**P-D-46**: 02 already places its write here, and until now the row enumerated only three writes, which left the `PublishDoor`, the freeze digest and §5's golden vector with no defined input set; the door writes, the owning slice registers the validators, and no third registration point is introduced); the `ProductHeadSaved`/`SkuHeadSaved` outbox row in the same transaction. Saves **never** touch `published_version` - `inst-fd-save-txn`
5. [ ] - `p1` - A save on a `draft`, `published` or `deprecated` head holding an open approval **invalidates it** — the Foundation raises the `approval-invalidated` hook (an in-process hook, **no broker event**); slice 05 owns re-queue semantics - `inst-fd-approval-hook`

### Discard a never-published draft

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-discard`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `actor_ref` resolution, authorize, and idempotency as at create - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - Legal only from `draft` with `published_version = 0`; transition to `discarded` (terminal); the `ReservationIndex` (§4.2) and the `product_code` index (§4.1) both exclude `discarded` rows, so the `skuCode`/`productCode` reservation releases by the same write; emits the `SkuDiscarded`/`ProductDiscarded` event - `inst-fd-discard`

### Publish an entity (the mechanics half)

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-publish`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `actor_ref` resolution, authorize, and idempotency as at create — the PRD names **publish** among the retried
verbs, and 04's crash-replay of a scheduled activation (04 `inst-sp-idempotent`) rides this store keyed by transition id - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - `PublishDoor` accepts `(entity, expected internal revision, optional corrected bucket-ii field and value)` — the revision arriving as the door's `If-Match` (P-D-33), and the third argument supplied **only** by 07's `CorrectionDoor`, which already accepts it (**P-D-41**: §4.2 admits a bucket-ii write only in the same statement as a `published_version` bump, which is this door's own head-row UPDATE, and 07 delegates its re-publish here — so without the argument the value 07 holds has no carrier into the statement that may write it. Additive: 06's composition-clear and 09's per-row publishes pass nothing and are untouched) — a `draft` for its first publish, or a `published`/`deprecated` **head** for version N+1 (a re-publish changes the version, never the state); stale revision fails `STALE_REVISION` — an approval is only usable against the exact revision it pinned (slice 05 stores the snapshot; the Foundation enforces the match) - `inst-fd-publish-pin`
3. [ ] - `p1` - Re-run the **full** pipeline at publish (shape, state, identity, every registered validator for `→ published` — which names the **target state, not the edge** (**P-D-32**), and which **P-D-34** reads as naming the *publish act* rather than the row's `lifecycle_state` afterwards — the door accepts a `deprecated` head for version N+1 and leaves it `deprecated`, so a state-after reading would select nothing there: a re-publish takes no edge, so an edge-keyed reading would run no validator at all and empty this fail-closed re-run, while also pulling the `deprecated→published` two-person ceremony onto a content re-publish that changes no state): an entity that stopped being publishable since approval fails closed `INCOMPLETE_ENTITY`/rule-named code, never publishes stale - `inst-fd-publish-revalidate`
4. [ ] - `p1` - The governance gate (slice 05) runs **inside** the door, and the door therefore carries an explicit **authorization mode** (Blocking 9 fix) - `inst-fd-governance-gate`
   - [ ] - `p1` - The two modes: `Gate` — the ordinary interactive publish, which needs a
     `satisfied` record — or **`PreAuthorized(approvalId)`**, the mechanical stage of a composite
     act. The mode is **an internal door argument, never a wire-visible parameter: the REST and SDK
     publish surfaces always call in `Gate` mode** (owner's call, 2026-08-27), so the re-use the
     mode admits is bounded by the in-process callers rather than by a grant (05
     `inst-gv-one-shot`: scheduled activation, a cascade leg, a bulk
     row) - `inst-fd-gate-mode`
   - [ ] - `p1` - **In `Gate` mode**, a publish with no satisfied, non-superseded `ApprovalRecord`
     pinned to the door's expected revision fails `APPROVAL_REQUIRED` (05 `inst-gv-gate`: "The gate
     never re-evaluates materiality at publish") - `inst-fd-gate-mode-gate`
   - [ ] - `p1` - **Under `PreAuthorized`** the gate does not look for a `satisfied` record and does
     not consume one; it **verifies** the named record, raising `APPROVAL_REQUIRED` only when the
     verification fails. Without the mode the two readings collide and every scheduled publish
     fails terminally: the runner drives "the ordinary Foundation publish door" (04
     `inst-sp-activate`), the gate inside it would see a `consumed` record, and 04
     `inst-ar-failure` wraps that into a terminal `SCHEDULE_STALE_APPROVAL`.
     **What "verifies" means has two arms (P-D-105).** For an act whose subject is the record's own
     subject, it is *"the record authorized **this** subject and its pinned revision still
     matches"*. For a **scheduled flip** it is *"the record is `consumed`, **and** the row being
     flipped names it in its own `approval_ref`"*, and subject/revision equality is **not** asked —
     because a cascade leg's subject is a **child** entity with its own revision while the record
     names the parent, and `products_approval` stores one subject and one revision per record, so
     the first arm fails for every leg by construction. The second arm is not a weakening: its
     operand is a stored column on a row no caller can write, every writer of
     `products_scheduled_transition` running the gate first, and that writer count is guarded in
     code. The arm is **scoped to that table** — `products_bulk_batch.approval_ref` has the same
     shape and different writers, so extending it to a bulk row is a separate
     decision - `inst-fd-gate-mode-preauthorized`
   - [ ] - `p1` - **Re-validation stays fail-closed in both modes** — the mode governs *who
     approved*, never *whether the entity is still
     publishable* - `inst-fd-gate-revalidation`
   - [ ] - `p1` - What crosses the seam: the Foundation knows only "the gate answered yes/no +
     reason, and on yes the authorizing `ApprovalRecord`'s id, plus whether that record carried the
     two-person uncomposed-bundle override (§4.2's `composition_pending` operand)" — the id being
     what
     `inst-fd-publish-consume` consumes and what §4.3's `approval_ref`
     stores - `inst-fd-gate-verdict`
   - [ ] - `p1` - An approval rejection "returns the entity to `draft`" (AC #26) reads: a
     first-publish entity stays `draft`; a published head keeps its pending edits unpublished — no
     state flip either way (design reading under the head-row model, **flagged**: the literal
     reading would need a `published→draft` edge the PRD's own forward-only rule forbids —
     registered in `PRD` §15 with the PRD owner); **no event** - `inst-fd-gate-rejection`
5. [ ] - `p1` - On yes, **all in one transaction**, in the order below - `inst-fd-publish-txn`
   - [ ] - `p1` - **Freeze first**, and freeze the **post-act image**: the door computes the content this act
     leaves behind — including the `composition_pending` value the same UPDATE is about to write —
     and freezes that (**P-D-33**: the row's own key already carries `published_version = N+1`, so
     freezing the pre-UPDATE image would store content the act never produced and would put the
     digest and 10's byte-for-byte restore drill on different bytes). That content (excluding the
     metadata map and the five columns §4.3 excludes — `lifecycle_state`, `deprecation_provenance`, `replaced_by_sku_id`, `internal_revision` and, since P-D-129, `correction_ref`) goes into `products_entity_version`, **then** `published_version += 1`
     (the door is this column's **only** writer) — the bump second because §4.2's whitelist admits
     it *only where the matching `products_entity_version` row for the new value exists*, so the
     reverse order trips the guard on every publish. On the `draft→published` edge the same
     head-row UPDATE also writes `lifecycle_state = 'published'` — the door owns that edge (§2),
     and it must be that one UPDATE rather than a second statement, because §4.2 bumps
     `internal_revision` "+1 on every admitted UPDATE, without exception" and two statements would
     bump twice against the "**once**" below. On a `bundle` SKU that same UPDATE also carries
     `composition_pending` — set where this publish carried the uncomposed-bundle override, cleared
     where it did not (§4.2, P-D-32). A re-publish takes no edge and leaves the state
     alone - `inst-fd-publish-freeze`
   - [ ] - `p1` - First publish makes the `skuCode`/`productCode` reservation **permanent** — immutability
     enforced by the trigger whitelist from this row-state
     on - `inst-fd-publish-reserve-permanent`
   - [ ] - `p1` - A **corrected bucket-ii value**, when the door was given one, is written by that
     same head-row UPDATE — the mechanism `composition_pending` already uses, and the reason the
     freeze above is the **post-act image**: the corrected value is what version N+1 must carry
     (**P-D-41**) - `inst-fd-publish-correction`
   - [ ] - `p1` - `internal_revision += 1`, carried by the **same head-row UPDATE** as the freeze step above
     rather than as a second statement — **once**: the publish door is one act rather than a
     transition plus a publish, so the transition guard's "every transition" does not reach the
     `draft→published` edge the door owns, and the invalidation hook does not fire on it either,
     the same transaction being the one that consumes the approval (owner's call, 2026-08-27,
     P-D-26). **Every** publish bumps it, first and re-publish alike, so the ETag moves whenever
     frozen content does and a stale client's cached representation can no longer pass its own
     precondition - `inst-fd-publish-bump`
   - [ ] - `p1` - Emit `ProductPublished`/`SkuPublished` - `inst-fd-publish-emit`
   - [ ] - `p1` - **Re-announce a retirement in flight** (**P-D-48**, the door P-D-20 lacked): where
     the entity holds a live retire intent — a pending retirement `ScheduledTransition`, 04
     `inst-rt-initiate` — the same transaction also enqueues `SkuRetired`/`ProductRetired` with the
     new `fromVersion`, the same `effectiveAt` and the same retirement identity. The event, its
     payload and that identity are 04's; the enqueue is this door's, and the row names the event it
     enqueues, so nothing is inherited or owed under P-D-34's act unit - `inst-fd-publish-reannounce`
   - [ ] - `p1` - Mark the gate's `satisfied` `ApprovalRecord` `consumed` (05
     `inst-gv-one-shot` requires the flip **in the same transaction as the authorized act**; nothing is consumed under
     `PreAuthorized`); **no event of its own** - `inst-fd-publish-consume`
6. [ ] - `p1` - Post-commit, slice 06 consumes the publish event **as content only** (what became publishable); an entity publish **never enqueues a CatalogVersion increment** — addressability comes from downstream requests or an operator catalog-publish act (06 `inst-cv-request`; M1 fix of the 06 review), and the Foundation itself requests nothing (06 `inst-cv-request`'s trigger set names pricing, this gear's slice-09 bulk commits and the operator act — not this slice) - `inst-fd-publish-fanout`

### Transition an entity (state-machine floor)

Declared by [`../features/foundation.md`](../features/foundation.md) §2 as `cpt-cf-bss-products-flow-transition`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `actor_ref` resolution, authorize, and idempotency as at create - `(cont. inst-fd-idempotency)`
2. [ ] - `p1` - The transition guard, which reaches **`lifecycle_state` changes only** — a save is not a transition, and a re-publish is not an edge (H1 fix: the head row is the authoring surface in every non-terminal state) - `inst-fd-transition-guard`
   - [ ] - `p1` - **The edge list.** The Foundation admits exactly: `draft→published` (door),
     `draft→discarded`, `published→deprecated`, `deprecated→published`, `deprecated→retired`;
     anything else fails `ILLEGAL_TRANSITION` - `inst-fd-transition-edges`
   - [ ] - `p1` - **Every transition bumps `internal_revision` and fires the approval-invalidation
     hook** exactly as a save does — **except any transition that consumes an approval in the same
     transaction — `draft→published`, which the publish door owns, and every gated edge P-D-30 put
     the gate phase on — which bumps once with no hook** (P-D-26, extended by **P-D-34** for the
     reason 05 C3 already gives: a hook firing against the record the act is consuming has no
     defined ordering, and P-D-30 reproduced that collision on `deprecated→published`) — (M-2 fix, slice-05 review:
     head-at-revision-N stays byte-identical to any approval snapshot pinned at N;
     transition-written columns cannot drift under a pin) - `inst-fd-transition-bump`
   - [ ] - `p1` - Policy conditions on the legal edges (two-person on un-deprecate, scheduled
     retirement, cascades) are slice 04/05 validators registered on the edge — the floor stays
     policy-free - `inst-fd-transition-policy-free`
   - [ ] - `p1` - **No event here** on `published→deprecated`, `deprecated→published` and
     `deprecated→retired` — 04 announces those three, except the **Product** side of
     `deprecated→retired`, which no slice announces (§4.5); `draft→discarded` emits its own event
     through the discard door (`inst-fd-discard`) - `inst-fd-transition-events`
3. [ ] - `p1` - `retired` and `discarded` are terminal at the physical layer too: the append-only trigger's whitelist admits no `lifecycle_state` write out of them - `inst-fd-terminal`

## 3. Processes / Business Logic

### 3.1 Validation pipeline

Declared by [`../features/foundation.md`](../features/foundation.md) §3 as `cpt-cf-bss-products-algo-pipeline`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Pipeline order — one pre-pipeline gate, then **seven** ordered phases - `inst-fd-pipeline-order`
   - [ ] - `p1` - **Authorization is not a phase**: it is a **pre-pipeline gate**, run before the
     pipeline opens (owner's call, 2026-08-27, P-D-30) — the only order in which a denied caller
     neither consumes an idempotency key nor writes a claim row, and the order §2's flows already
     use ("`actor_ref` resolution, authorize, and idempotency as at create") — the ref ahead of the
     gate because an authorization denial is itself a refusal and §4.4's audit row for it carries a
     non-nullable `actor_ref`. Its refusal code is
     slice 05's, RBAC grants being that slice's (§1.5) - `inst-fd-pipeline-authz`
   - [ ] - `p1` - **The seven ordered phases**: **idempotency
     resolution** (the first pipeline phase of every mutating flow **that carries an `Idempotency-Key`** —
     skipped, never failed, on a keyless request; §2, P-D-34) → **precondition** (`If-Match` on a head
     verb, the pinned revision at the publish door) → **shape** (types/formats/required-at-this-state,
     and **resolvability of every reference the payload carries** — an unresolvable `productId` is a
     defect of the payload, which is why it rides `VALIDATION`) → **state** (the §2 edge list,
     bucket routing for a published-state field write, `cloned_from`'s create-only rule, the parent's
     own **terminal** state (`PARENT_TERMINAL`; the parent-*published* check is 04's registered
     validator, a different phase), and the **subject's own** terminal state (`ENTITY_TERMINAL` on a
     `retired`/`discarded` head) —
     everything judged from the row as it now stands rather than from the payload, and therefore
     after the reference that names it has resolved; owner's call, 2026-08-27, P-D-24) →
     **identity** (uniqueness, reservation, containment) → **registered validators** → **governance
     gate** - `inst-fd-pipeline-phases`
   - [ ] - `p1` - Inside **registered validators**: each slice contributes `RegisteredValidator`s
     keyed by kind + transition/target-state/field-set; execution order is registration order within the phase,
     and no rule may read another rule's verdict - `inst-fd-pipeline-validators`
   - [ ] - `p1` - The **governance gate** phase hosts **any gated act, not publish alone** (owner's
     call, 2026-08-27, P-D-30: 05 words the obligation generically, "submit → quorum →
     publish/apply" over both entity publishes and `GovernedLiveOp`s, and 04's un-deprecation is
     two-person with a slice-05 gate registered on that edge, so a transition door consumes the
     `satisfied` record exactly as the publish door does; scoping the phase to publish left that
     ceremony with a gate no phase hosted). The phase runs at **every** mutating door and passes
     trivially where the act is ungated (**P-D-34**): a head save is ungated —
     `inst-fd-approval-hook` has it *invalidate* an open approval, never consume one — so `Gate`
     mode imposes no approval requirement on create, save or discard - `inst-fd-pipeline-gate-phase`
2. [ ] - `p1` - Fail-closed and atomic: the phases run in the order above and the run **stops at the first
failing phase**, collecting violations per-field *within* that phase into one rejection
(**P-D-33**: §4.4's audit row carries a single `error_code`, so collecting across phases would
produce more codes than the row can record). **The rejection the caller receives carries every
violation that phase collected; the audit row records one code** (**P-D-37** — the donor's split,
measured: `gears/bss/pricing` renders a whole `ValidationReport` into one refusal, and audits no
validation refusal at all, while this gear audits every one of them under `nfr-availability-audit`
and so needs a single code per row). Where a phase can collect more than one **code**, the row
records the first by the precedence §3.3 states for that phase. Only the `state` phase can:
`shape` raises one code with many per-field entries, and `identity` is decided under the write and
can return only one (§3.4). Any failure rejects the whole mutation with an
audited reason; there is no partial application anywhere in the gear (PRD AC #38) - `inst-fd-fail-closed`
3. [ ] - `p1` - Registration is compile-time code (a slice ships its validators with its handler); the pipeline exposes `rule_names()` for observability only — attribution in rejections rides the **error code**, never the rule name - `inst-fd-rule-registry`
4. [ ] - `p1` - Field-mutability enforcement frame (raised from `p2` by the owner 2026-08-: the physical guard routes by these tags and `ILLEGAL_FIELD_MUTATION` ships in the p1 contract, so the classification cannot be later than the things that read it) - `inst-fd-mutability-frame`
   - [ ] - `p1` - Each published-state field carries a **bucket tag** — i structural / ii
     correctable / iii material-mutable / iv descriptive (PRD
     `fr-field-mutability-matrix`) - `inst-fd-bucket-tags`
   - [ ] - `p1` - The Foundation **refuses bucket-i writes after first
     publish** - `inst-fd-bucket-i-refusal`
   - [ ] - `p1` - It **refuses a bucket-ii write at the head door after first publish, naming slice 07's
     correction door in the reason**, rather than forwarding it (owner's call, 2026-08-27: one door, one
     effect — a single call must not silently pass through two ceremonies with different grants;
     the **application** enforces which door, the physical guard backing it by refusing any column
     that moves outside its admitted state — P-D-31). Bucket-ii writes after first publish are admitted **only through
     slice 07's correction door** — door identity being an **application** guarantee, the physical
     guard carries the interim row-image predicate §4.2 pins for those columns (P-D-34), with a
     tighter one still **owed by 07** - `inst-fd-bucket-ii-refusal`
   - [ ] - `p1` - Bucket iii/iv are ordinary head-row saves re-published as version N+1, their
     materiality judged by slice 05 - `inst-fd-bucket-iii-iv`

### 3.2 Idempotency store

Declared by [`../features/foundation.md`](../features/foundation.md) §3 as `cpt-cf-bss-products-algo-idempotency`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - The idempotency record and what a replay reproduces - `inst-fd-idem-replay`
   - [ ] - `p1` - **Key scope `(tenant_id, endpoint, client_key)`**, where a wire caller's
     `endpoint` is the **concrete resource path** and not the route template (**P-D-42**): under
     the template two publishes of different entities under one client key share the whole key and
     an identical (empty) body hash, and the second would replay the first's 200 without running —
     the path id being in neither the body nor, since P-D-34, the hash. The two lanes therefore
     name their subject in different components of the key, which is stated here once. A caller
     with no wire surface
     writes a **reserved lane name** in `endpoint` (`internal:scheduled-activation`,
     `internal:cascade-leg`, `internal:bulk-row`; owner's call, 2026-08-27, P-D-26) and its own id
     in `client_key` — the transition id for a scheduled activation, the leg's for a cascade, the
     row's for a bulk row — so two internal lanes cannot collide on one key and the `internal:`
     prefix cannot collide with a wire endpoint - `inst-fd-idem-key-scope`
   - [ ] - `p1` - **Stored**: payload hash + the stored response (`response_status`,
     `response_body`). **The hash is over a canonical rendering of the *parsed* request, not over
     the received bytes** (owner's call, 2026-08-27; the donor's D-174 puts it this way: the digest
     itself "is internal to the gear and never crosses the wire", and what differs on the wire is
     which of the two readings a client experiences): a client that re-serialises its JSON with a
     different key order on retry is making the same request, and a byte hash would answer it
     `IDEMPOTENCY_CONFLICT` instead of replaying the outcome — breaking idempotency exactly where
     it is needed. The canonical rendering is §4.3's, so the gear pins one such rule and not
     two. The hash covers the **body's** present fields and **not the precondition** (**P-D-34**):
     `If-Match` is a header, and a client that was refused `STALE_REVISION`, re-read the head and
     retried is making the same request — hashing the precondition in would answer that retry
     `IDEMPOTENCY_CONFLICT` instead of running it - `inst-fd-idem-hash`
   - [ ] - `p1` - **Identical replay** returns the stored outcome without touching entities,
     versions, or the outbox; **no event** - `inst-fd-idem-replay-outcome`
   - [ ] - `p1` - **A different payload under a live key** fails `IDEMPOTENCY_CONFLICT` — never a
     silent no-op - `inst-fd-idem-conflict`
2. [ ] - `p1` - **The row is `claimed` or `answered` and nothing else, and the claim INSERT is the gate** (owner's call, 2026-08-27, adopting the donor's model whole — `gears/bss/pricing/docs/design/01-foundation.md` §3.7) - `inst-fd-idem-claim`
   - [ ] - `p1` - The door inserts the key `claimed` with **`payload_hash` stamped from the parsed request**
     (the §4.3 rendering §3.2 reuses; the column is non-nullable and `inst-fd-idem-claim-inflight`
     compares against it) and both response columns null **before** the
     guarded operation, and sets `state = answered` with **`response_status` and `response_body`**
     together on completion. **That write joins the mutation's transaction, and since P-D-42 so
     does the claim itself** (**P-D-34**, narrowed by **P-D-38** and **P-D-42**): outside it, a
     crash between the mutation's commit and the answer would leave a `claimed` key surviving its
     own act, and the retry would re-execute a committed one. Inside it there is no such gap —
     claim, mutation and answer commit together or not at all, which is also why a refusal leaves
     nothing behind (owner's call, 2026-08-27, P-D-29: the donor's two columns, adopted
     after all — this gear had imported a single `outcome_ref` instead, and a replay must reproduce
     the original response *including its status*, which a bare reference to an entity cannot do
     and which a refusal has no entity to reference at all); **no
     event** - `inst-fd-idem-claim-write`
   - [ ] - `p1` - **The claim joins the mutation's transaction** (owner's call, 2026-08-28,
     **P-D-42**, superseding P-D-26's arm, whose stated reason was measured and does not hold).
     That reason was that a claim inside the mutation's transaction would be "invisible to the
     concurrent duplicate the row exists to refuse" — but **the gate is the insert, not a lookup**:
     the duplicate's own INSERT conflicts with the winner's uncommitted row and waits, then either
     finds the committed answer and replays it, or finds nothing left to conflict with — the winner
     having rolled back — and claims the key itself. Visibility is never required; the unique index
     does the work. On SQLite the loser is answered `SQLITE_BUSY` rather than blocking, so the door
     carries a busy timeout and retries: the guarantee is identical — two are never admitted — and
     only the waiting differs. **One composite wire act extends this** (**P-D-72**): the
     product-with-SKUs clone's claim joins the *parent's* transaction — the composite's first — and a
     committed-but-unanswered claim there means *in progress: resume*, the retry re-entering the
     family act and skipping sources already cloned, never replaying and never refusing - `inst-fd-idem-claim-txn`
   - [ ] - `p1` - A duplicate **whose payload hash matches the claimed key's** arriving against a
     `claimed`, unanswered key is refused **`IDEMPOTENCY_KEY_IN_FLIGHT`** (409) — without this
     state such a duplicate matches neither branch of `inst-fd-idem-replay`, because a stored
     response cannot exist before the operation has produced one, and a retry storm is exactly the
     concurrent case. A payload *mismatch* is the branch `inst-fd-idem-replay` already answers, and
     stays `IDEMPOTENCY_CONFLICT` in either state. A request refused `IDEMPOTENCY_KEY_IN_FLIGHT`
     writes **nothing** to the row — it does not own the key. **Two paths reach this refusal and they
     are not equally likely** (**P-D-49**, stated as the donor states it): *meeting an unanswered
     fresh claim* is **unreachable** under `inst-fd-idem-claim-txn`'s one-transaction contract — the
     holder has not committed, so the duplicate is still blocked on the index and, when released,
     meets either the committed answer or nothing left to conflict with — so reaching it that way
     means the contract was violated, and refusing is how that becomes visible instead of becoming a
     fabricated reply; *losing the expired-key takeover race* (item 3 below) is **reachable in
     production with no contract violation by anyone**, and is what keeps the code live - `inst-fd-idem-claim-inflight`
   - [ ] - `p1` - **A refusal stores nothing and releases the key** (owner's call, 2026-08-28,
     **P-D-38**, adopting the donor's posture after measuring it — `gears/bss/pricing`'s
     `idempotent.rs` runs claim and answer inside the mutation's transaction so that "a failure
     anywhere rolls the claim back with the mutation", and it stores no refusal at all). The answer
     write joins the mutation's transaction and rolls back with it, so a refused request leaves no
     stored outcome, **and the claim rolls back with it** (**P-D-42**: the claim shares that
     transaction, so nothing survives to delete), freeing the key immediately; a retry **runs**. An idempotency key exists to prevent a duplicate
     *side effect*, and a refusal has none — the mutation rolled back — so storing one protects
     nothing while freezing a transient verdict for `expires_at`'s window, at least a day. **There is
     no carve-out**: `AUDIT_UNAVAILABLE` needed one only because refusals were stored. Measured
     against the alternative: keyed on "the verdict can change on retry", the exception selects
     **ten of the taxonomy's fifteen codes** — every one an operator or a sibling act can clear —
     and the exception becomes the rule - `inst-fd-idem-claim-refusal`
   - [ ] - `p1` - `claimed` therefore means exactly "in flight", and **no row is ever left
     needing release**: a claim that is not answered was rolled back with its mutation, so there is
     nothing committed to expire. The **`in_flight_until`** column and its deadline are removed
     (**P-D-42**) — the operand this slice could not pin, because no door timeout exists anywhere
     in the set to derive it from, turns out not to be needed at
     all - `inst-fd-idem-claim-inflight-until`
3. [ ] - `p1` - Retention: `max(24h, max_freeze_timeout)` read from config (C6). **Expiry is evaluated at claim time, not by a reaper** (same call, same donor): a claim against an expired key succeeds and replaces it — **as a compare-and-swap on the held row's own claim stamp** (**P-D-49**, the donor's `take_over`): nothing holds an expired row between one transaction's conflict check and its takeover UPDATE, so two duplicates on one expired key both clear the check and both read the same expired row, and without the predicate both would be told they claimed it and the guarded mutation would run **twice** under one key. The UPDATE therefore carries `WHERE <the stamp the reader saw>`; exactly one matches, and the loser is refused `IDEMPOTENCY_KEY_IN_FLIGHT` having executed nothing — it may even carry a different payload from the winner, and is still refused in-flight rather than for the mismatch, since this transaction never compared the two. So correctness never waits on a sweep; a background sweep still runs, but only to reclaim space. Expiry never retro-invalidates an outcome. `expires_at` is **stamped at the claim INSERT** from C6's configured value (**P-D-34**: the column
is non-nullable and the row is inserted `claimed`, so it cannot wait for the answer); it is the
**retention** window of the key. Nothing releases a crashed claim, because since **P-D-42** a
crashed claim does not commit: it rolls back with the mutation it shares a transaction with;
**no event** - `inst-fd-idem-retention`

### 3.3 Error taxonomy (Foundation-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-error-taxonomy`

`DUPLICATE_NAME`, `DUPLICATE_CODE` (either reservation — `skuCode` or `productCode`),
`STALE_REVISION`, `IDEMPOTENCY_CONFLICT`,
`IDEMPOTENCY_KEY_IN_FLIGHT`, `ENTITY_TERMINAL` (**any head write on a `retired`/`discarded` row** — save,
publish or correction alike: the subject's own terminal state refusing the write, as
`PARENT_TERMINAL` is the parent's; owner's call, 2026-08-27, P-D-25, widened from the save-only
form by **P-D-32** because the publish door's accepted set excludes a `retired` head and
`ILLEGAL_TRANSITION` cannot cover it — a re-publish is not an edge), `AUDIT_UNAVAILABLE` (a refusal's **or an elevated read's** audit row could not be
written, §4.4 — one of the gear's **three** 503s, alongside 08's `READ_MODEL_OVERLOADED` and 03's `USAGE_TYPE_UNAVAILABLE`),
`ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION` (a write the head door may not take: bucket-i after first publish, any UPDATE of
`cloned_from` (stricter than bucket-i — §4.1/§4.2), or
bucket-ii after first publish, which belongs to 07's correction door — the reason names the door; 07's
structural-identity attempts ride this code rather than declaring their own), `SCOPE_NOT_CONTAINED`,
`PARENT_NOT_PUBLISHED` (registered by slice 04 on the `→ published` **target state** under
**P-D-32**, which 04 `inst-pc-ordering` now words as the target state; named here so AC #38's
map is complete), `PARENT_TERMINAL` (the parent's own state, wherever the `state` phase runs), `INCOMPLETE_ENTITY`, `APPROVAL_REQUIRED` (raised through the governance
gate), `VALIDATION` (per-field envelope), `RETIREMENT_PENDING` (the create door's parent guard,
`inst-fd-containment-retire-intent` — **declared by slice 04**, listed here only for the response map
(**P-D-34**: P-D-30 gave 04 **both arms**, so this slice raises neither and cannot hold the
declaration — a call about this code, not a general test. **P-D-35** fixes the general rule: *the
slice that names a code for its response map holds the declaration unless the register moves it*,
which is why `PARENT_NOT_PUBLISHED` stays this slice's and `RETIREMENT_PENDING` does not), so this code has two raising **arms**, and both are 04's).

**A code belongs to the rule that raises it, and the rule belongs to a slice** (owner's call,
2026-08-28, **P-D-36**). §3.1's seven phases stay as the **execution order** — what runs before
what, and therefore which refusal a caller meets first — and stop being a taxonomy: no code is
required to belong to exactly one of them, and there is no carve-out list, because there is no
longer a rule to carve anything out of. This is the donor's shape, adopted after measuring it:
`gears/bss/pricing`'s shared `ValidationPipeline` registers rules and collects a report, its codes
are `const`s on the rules that raise them, and it carries no notion of a validation stage at all —
`phase` in that gear names a plan phase and nothing else. The phase-shaped attribution this section
used to carry was this set's own invention, and the contradictions it produced — a `VALIDATION`
that had to come from two stages at once, a carve-out list that closed at two or at zero depending
on which sentence you read — were properties of the invention rather than of the gear.

**The AC #38 map therefore keys on code → declaring slice.** Which slice declares a code is fixed
by **P-D-35** — the slice that names it for its response map holds the declaration unless the
register moves it — and 12 `inst-cc-errors` lints that pair. The slice unit buys what the phase
unit was introduced to buy and the door unit could not: **P-D-24** abandoned the door unit because
one code is raised at many doors, and a code has exactly one declaring slice by construction.

Codes raised outside the pipeline need no special status under this model and get none.
`CONTENT_PII_BLOCKED` is raised by 02's `inst-av-pii-block` hook, which every door carrying a
free-text `reason` invokes (02 `inst-av-pii-reason` enumerates them) and which slice 02 declares.
`AUDIT_UNAVAILABLE` is raised by §4.4's audit-write path when a refusal's own row cannot commit —
the one code here raised *after* a decision has been reached — and this slice declares it. 05's
owed authorization-denial code and `BREAKGLASS_WRITE_FORBIDDEN` are 05's. That is the whole of what
has to be said about any of them, and it is what replaces three passes of carve-out arithmetic.

**Where each check runs is still stated, because the order decides which refusal comes first**: the **identity** phase raises the
uniqueness, reservation and containment codes (`DUPLICATE_NAME`, `DUPLICATE_CODE`,
`SCOPE_NOT_CONTAINED`) wherever it runs — create, save, and the publish re-run — the
**state** phase raises `ILLEGAL_TRANSITION` (the edge list), `ILLEGAL_FIELD_MUTATION` (bucket
routing, and `cloned_from`'s never-in-any-UPDATE rule of §4.1/§4.2, which bites while
`published_version = 0` where bucket-i does not), `PARENT_TERMINAL` (the parent's own state) and `ENTITY_TERMINAL` (the subject's own)
wherever it runs — and one act can satisfy more than one of them, a save on a `retired` head that
also moves a bucket-i column satisfying `ENTITY_TERMINAL` and `ILLEGAL_FIELD_MUTATION` alike, so
**the audit row records them in this precedence** (**P-D-37**): `ENTITY_TERMINAL` →
`PARENT_TERMINAL` → `ILLEGAL_TRANSITION` → `ILLEGAL_FIELD_MUTATION`, running from the refusal that
admits no write to the row at all down to the one that refuses a single column. The caller's
rejection carries all of them regardless; the precedence governs the one code the row stores — the
**shape** phase raises `VALIDATION`, the **precondition** check raises `STALE_REVISION` at both
the `If-Match` verb and the publish pin, idempotency resolution raises `IDEMPOTENCY_CONFLICT` and
`IDEMPOTENCY_KEY_IN_FLIGHT` at every door that resolves a key, and the **registered validators** phase raises `INCOMPLETE_ENTITY` and the
rule-named codes wherever it runs, including the publish re-run, and the **governance gate**
phase raises `APPROVAL_REQUIRED` at every gated act, publish or transition alike;
slice-owned codes (taxonomy cycles, unit rules, freeze, bulk rows…) are declared in their
slices, and slice 12's coverage check completes the AC #38 ↔ code ↔ slice map.
`RETIREMENT_PENDING` is raised by **slice-04 validators at two doors** — the create door
(the owner call put the operand there rather than in the Foundation's identity phase) and
the un-deprecation edge — and 04 declares it.

Two declarations follow from the slice unit rather than from a phase count. `SCOPE_NOT_CONTAINED`
stays declared here because 04 C5 is "the final form of 01's interim check" in that slice's own
words rather than a second raiser, which **P-D-34** reads literally: 04's final rule **replaces the
operand inside this slice's `identity` phase** and is not registered as a slice-04 validator. And
`ILLEGAL_FIELD_MUTATION` stays declared here because 07's structural-identity attempts "ride 01's"
code rather than declaring their own. Codes are
part of the SDK contract; renames are breaking.

**Problem responses (RFC 9457):** `APPROVAL_REQUIRED` (403); `DUPLICATE_NAME`, `DUPLICATE_CODE`, `IDEMPOTENCY_CONFLICT`, `IDEMPOTENCY_KEY_IN_FLIGHT`, `PARENT_TERMINAL`, `PARENT_NOT_PUBLISHED`, `RETIREMENT_PENDING`, `STALE_REVISION`, `ENTITY_TERMINAL`, `ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION` (409); `AUDIT_UNAVAILABLE` (503); `SCOPE_NOT_CONTAINED`, `INCOMPLETE_ENTITY`, `VALIDATION`, `CONTENT_PII_BLOCKED` (422).

*`ILLEGAL_TRANSITION` and `ILLEGAL_FIELD_MUTATION` moved 422 → **409** by P-D-32: all four codes
the `state` phase raises are refusals by the row's **current state**, which is this block's own
409 rule, and splitting them left one phase straddling two status classes. **That straddle is a
rationale, not an invariant** (P-D-33): the `identity` phase legitimately spans 409 and 422, and
moving `SCOPE_NOT_CONTAINED` to 409 would contradict this block's own "422 for content the door
cannot process". Wire-visible — a 422
reaches the wire as 400 and a 409 as 409 — and taken while nothing is built.*

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
class is not "checked against it". **The 422s here are architectural, not wire** — see the rendering note below, which quotes
the sibling plan-price gear's rule: no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `CONTENT_PII_BLOCKED`
  (slice 02) and `RETIREMENT_PENDING` (slice 04, P-D-34) — the status is repeated, not a second declaration, so the one-declaration rule
  stands.*

**Status rendering — the 422s in this set are architectural, not wire (normative).**

The `422` annotations in every slice's problem-response block say *unprocessable content*: the
request was understood and the registry refuses it. They are **not** a wire status. The platform's
`CanonicalError` model has no 422 category — `libs/toolkit-canonical-errors/src/problem.rs` carries
no 422 arm — so, absent a transport override (which neither this design set nor pricing's
declares anywhere), every architectural 422 in this design set reaches the wire as a **400
carrying its wire code**, and **the code string is the discriminator a consumer matches on, not the status**.
An endpoint **MUST NOT** declare a 422 response for an error **carrying a registry code** in its
`OpenAPI` registration — **the rule stands as this gear's choice, not as an impossibility**
(owner's call, 2026-08-27). It read "because no path can produce one" until that call, and that
was false: 400 is the *default* for
`InvalidArgument`/`FailedPrecondition`/`OutOfRange`, and `docs/arch/errors/DESIGN.md` §2.2 lets a
single occurrence override the wire status so long as it stays in the same class — 422 is in that
class, and the toolkit's own `extract::Json` path takes it. What makes the rule true here is that
**this gear declares no transport override anywhere, and neither does pricing** — so every
registry code has exactly one wire shape, which is the property the rule is protecting and the
reason it is worth keeping once its false premise is gone. **The framework layer is the exception and is not covered by it**: a handler taking
`toolkit::api::rest::extract::Json<T>` can still answer 422 on a schema violation, which is why the
toolkit ships `OperationBuilder::error_422` and tells an operation to register it individually
(`libs/toolkit/src/api/operation_builder.rs`). A schema violation carries no registry code — though it **is** a canonical error: the toolkit
builds one (`json_rejection_to_canonical`, `libs/toolkit/src/api/rest/extract/error.rs`) and
renders it 422 as the canonical `invalid_argument` category — on the wire the `type` is
`gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~`, the symbol naming it in the
toolkit being test-only. The **quotation below** is the sibling plan-price gear's rule verbatim; the canonical scoping and the framework exception above are this gear's own, added because pricing states the rule unscoped and the toolkit contradicts the unscoped form, and it is quoted rather than
paraphrased: `gears/bss/pricing/docs/design/01-foundation.md` §3.3 — *"The platform's
`CanonicalError` model has **no 422 category** at all (`InvalidArgument`, `FailedPrecondition` and
`OutOfRange` all render **400**), so every architectural 422 in this design set — here and in every
slice — reaches the wire as a **400 carrying its wire code**"*. Two consequences bind the implementation, the first from the rule quoted above and the second
from the same section's pagination rule, whose subject there is an undecodable cursor **and, in the
same sentence, an absent precondition (pricing D-141) — where this gear diverges: P-D-33 routes an
absent `If-Match` to `VALIDATION`, §2** —
*"a **malformed request** … answered 400 with no code of its own"*: a refusal is classified by
what it **is**, so a retriable
conflict on mutable state stays a **409** rather than collapsing into the 400 bucket; and a bare
**400 with no code of its own** is reserved for a malformed request, which is why no registry code
is mapped to 400. **A 404 is bare on the same reading** (**P-D-35**): a path segment is judged
before the pipeline opens, so no phase raises it, the governing `api-contracts.md` pins no code
for it, and no rule raises it at all — a path segment is resolved before any rule runs. Stated here
once, in the Foundation, rather than per occurrence.

### 3.4 Concurrency doors (PRD §6.13 residents of this slice)

Declared by [`../features/foundation.md`](../features/foundation.md) §3 as `cpt-cf-bss-products-algo-concurrency`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `skuCode`/`productCode` race: decided by the `ReservationIndex` (§4.2) and §4.1's `product_code` partial unique index under the insert **or the admitted bucket-i UPDATE** (§4.2), not by a read (two concurrent creates, or a create against a concurrent bucket-i change: exactly one admitted). **Because the index is the arbiter, a write violating two of them returns
one violation** — whichever constraint the engine checked first — so the `identity` phase cannot
collect a second code, and §3.1's per-field collection is a property of the **read-decided** phases
(**P-D-37**, measured against the donor, which folds every unique violation into a single error
without distinguishing which index fired) - `inst-fd-code-race`
2. [ ] - `p1` - Name race: decided by §4.1's partial unique index under the insert **or the admitted rename** (`name` is bucket-iii, §4.1), not by the step-3 read (two concurrent creates of one normalized name, or a create against a concurrent rename: exactly one admitted; the loser fails `DUPLICATE_NAME`) - `(cont. inst-fd-name-unique)`
3. [ ] - `p1` - Draft race: decided by `If-Match` (two editors: second fails `STALE_REVISION`) - `(cont. inst-fd-etag)`
4. [ ] - `p1` - Publish-vs-edit race: the door's pinned-revision check makes "approve rev N, publish rev N+1" impossible by construction - `(cont. inst-fd-publish-pin)`

## 4. Data / Storage (normative shape; DDL in migrations)

### 4.1 `products_product`

`product_id` (PK, uuid) · `tenant_id` · `brand_id` · `name` · `name_normalized` ·
`product_code` (nullable) · `lifecycle_state` (`draft|published|deprecated|retired|discarded`) ·
`internal_revision` · `published_version` · `deprecation_provenance` (nullable
`direct|cascaded`, slice 04) · **category assignments live ONLY in slice 02's `products_product_category`** (the assignment
table with the exactly-one-primary partial index — a second inline representation here would
be a divergence channel with no authority rule; the frozen version content carries the
assignment set as a copy at publish, like every other content class) · `region_scope` /
`brand_scope` (flat value sets, **NOT NULL, default the empty set**, where **the empty set means
*unrestricted*** rather than *nothing* — **P-D-39**: the column then has one spelling of absence
and one meaning for it, and a Product created without naming its markets sells everywhere rather
than refusing every child that names one) · `created_by` (pseudonymous ref) · `cloned_from` (create-only, immutable —
slice 11) · timestamps.

Indexes/guards: **partial UNIQUE `(tenant_id, brand_id, name_normalized) WHERE lifecycle_state
<> 'discarded'`** (P-D-04; discard releases the name exactly as it releases codes — **confirmed by the owner
2026-08-**, no longer a design-introduced residue: the PRD releases codes on discard and is
silent on the name, and holding the name would let one typo in a never-published draft burn it
forever. The asymmetry with `retired`, which *does* hold its name, is the intended one — a
discarded draft was never published and a retired entity was); partial UNIQUE on `(tenant_id, product_code) WHERE
product_code IS NOT NULL AND lifecycle_state <> 'discarded'`; append-only trigger enforcing the
shared head-row guard (§4.2), under which `product_code` is immutable once
`published_version > 0` exactly as `sku_code` is (PRD AC #1 puts an optional `productCode` under
the same rules as `skuCode`, and the guard named only the SKU column).

**Bucket assignment for the Foundation-owned columns** (owner's call, 2026-08-27 — the PRD's
matrix names `skuCode`/`productCode` and the SKU→parent link only as draft-editable, assigns no
named bucket to any Foundation column after publish, and routes "other **descriptive** fields" to bucket iv
(`fr-field-mutability-matrix`'s own wording; AC #2 words the same bucket as "other fields", and
**P-D-34 reads the FR's wording as governing**, which is why row identity sits outside the scheme
rather than in the catch-all)): `name` and `name_normalized` are **bucket-iii** — a published Product
can be renamed,
and the rename comes out as version N+1 under governance rather than forcing retire-and-clone;
`region_scope` and `brand_scope` are **bucket-iii in both directions**, widening and narrowing
alike, so a narrowing that would orphan a live child meets `fr-parent-child-integrity`'s fail-closed check in the
registered-validators phase, ahead of the governance gate (§3.1); `sku_code`, `product_code` and **`brand_id`** are **bucket-i** — the second following AC #1's
"under the same rules", and `brand_id` because re-branding moves the row into a different
`(tenant_id, brand_id, name_normalized)` scope, the very key §4.1's partial unique index enforces
on, and 11 states "a clone never retargets brand" (owner's call, 2026-08-27); and `cloned_from` is **stricter than bucket-i** — writable only in the
creating statement and never again, not merely never after first publish, so the lineage stays
evidence rather than a claim. A SKU's parent link `product_id` is **bucket-i** (owner's call,
2026-08-27): re-parenting changes *whose* SKU it is, not how it is described,
which puts it with identity rather than with governed content — so a mis-parented published SKU is
corrected by retire-and-clone, and nothing in the gear re-parents one today.

**`normalized(name)`** (the uniqueness and promotion-identity operand, P-D-04/AC #33a) is pinned:
Unicode NFKC → full casefold → trim + collapse internal whitespace to single spaces, computed
**application-side** so both engines store identical bytes.

### 4.2 `products_sku`

`sku_id` (PK, uuid) · `tenant_id` · `product_id` (FK) · `sku_code` · `type`
(`product|service|bundle`) · `lifecycle_state` · `deprecation_provenance`
(nullable `direct|cascaded`, slice 04) · `sellable` (default `true`, pricing D-46) · `plan_tier` ·
`tax_category_ref` · `gl_code_ref` (**both columns are contingent** — PRD §15 carries the
open question of whether this registry owns them at all, §2.1 saying they are owned elsewhere
while `fr-accounting-codes` requires the registry to persist and validate them) · `metering_unit` · `usage_type_ref` · `correction_ref` (nullable uuid — **P-D-129**, landed 2026-09-04: the door identity of 07's correction re-publish, written only in the same statement as the `published_version` bump and read by the bucket-ii predicate below) ·
`composition_pending` (bool, **NOT NULL, default `false`** — **P-D-35**: the create flow writes it nowhere and the publish door on a `bundle` is its only raiser, so the default is the unraised state; slice 06 semantics) · `replaced_by_sku_id` (slice 04) ·
`internal_revision` · `published_version` · `region_scope`/`brand_scope` (same shape and default as §4.1's, **contained in the parent's** per §2's flow; **the create door copies the parent's when the payload omits them** — P-D-39) ·
`created_by` · `cloned_from` (nullable; written only in the creating statement and immutable from then on —
stricter than bucket-i, which bites only after first publish; a later write fails
`ILLEGAL_FIELD_MUTATION`; slice 11) · timestamps.

Column ownership: the Foundation owns identity/lifecycle/version/scope columns; every
capability column above is **carried** here (one table, one row identity) but its write rules
are the owning slice's registered validators — the split is by validator, not by table.
**`ReservationIndex`: partial UNIQUE `(tenant_id, sku_code) WHERE lifecycle_state <>
'discarded'`** — atomic reservation at insert, release on discard, permanence after first
publish enforced by the trigger whitelist making `sku_code` immutable once
`published_version > 0`.

**Shared head-row guard (both entity tables; H1/M1 fix):** frozen
`products_entity_version` rows admit **no UPDATE ever and exactly one DELETE** — the referential
arm of §4.3 (**P-D-40**), not the audit table's row-image one. **The guard judges the data, never the door** (owner's call, 2026-08-27, P-D-31: a session
variable exists on Postgres and not on SQLite, so a door-reading guard breaks C1 in both halves —
dual-engine and "guards defined once". Which *door* performed a write is an **application**
guarantee; what the trigger enforces is which column may change in which state. §3.1's "one door,
one effect" therefore rests on the application layer, with the guard as the backstop that no
column moves outside its admitted state.) On head rows the trigger whitelist
admits exactly: `lifecycle_state` along the §2 edge list; `published_version` only as **+1**, and
only where the matching `products_entity_version` row for the new value exists;
bucket-iii/iv columns **only while `lifecycle_state` is non-terminal**;
`internal_revision` **+1 on every admitted UPDATE, without exception** — every save, transition,
correction and publish bumps it (`inst-fd-transition-bump`), so scoping it to the save door alone
refused writes the design requires; **`deprecation_provenance` (both tables) and `replaced_by_sku_id` (`products_sku` only — the
column names a SKU, and 04 `inst-rt-replacedby` requires `replacedBy` to name a `published`
SKU) only through
slice 04's acts** — and, until 04 supplies a tighter one, under
row-image predicates this slice pins now (**P-D-34**): `deprecation_provenance` changes **only in
the same statement as a `lifecycle_state` change** (04's writer is the deprecation transition), and
`replaced_by_sku_id` is **write-once per retirement, not per row** (**P-D-49**): `null` → non-null
at retirement initiation, and non-null → `null` in the same statement as the **governed cancel** of
that retirement's `ScheduledTransition` (04 `inst-lc-undeprecate`) — never any other change. Without
the second arm a SKU that was retirement-initiated, cancelled and un-deprecated stayed `published`
while permanently naming a successor no admitted write could clear, which the read surface then
resolved transitively (04 `inst-rt-replacedby`: "Validated once, and the row is terminal at the
flip …" — terminal at the flip, which a cancelled retirement never reaches). Door
identity remains an application guarantee, and a tighter predicate is still **owed by 04** (deprecation/cascade and retirement-initiation respectively — they
are neither save-door nor bucket-iii/iv columns, and leaving them unnamed either refused the
writes slice 04 specifies or dropped them to bucket iv, where an ordinary operator save could
re-stamp the provenance operand `inst-lc-provenance-reversal` reads — item 18 of the review), and **`composition_pending` (`products_sku` only — `bundle` is a value of the SKU-only
`type` column) set by the `PublishDoor` on a `bundle` publish that carried the
two-person uncomposed-bundle override** (owner's call, 2026-08-27, P-D-30: the door cannot know
whether plan-price has composed the bundle — that is 03's validator's judgement, refused
interactively as `BUNDLE_OVERRIDE_REQUIRED` — but it does see whether *this* publish carried the
override, and that is the Foundation-visible operand. It also removes the re-raise: 06's clearing
re-publish is a `system_signal` subject (05, P-D-14) carrying no override, so the predicate is
false and the flag stays cleared) (03 `inst-cl-bundle-override`;
`fr-bundle-adoption-guard` makes the flag a MUST on that publish) and
**changed only in the same statement as a `published_version` bump** — which admits both the
door's set and 06's clearing act (`inst-cc-clear`: a **system save + re-publish** of the head)
and refuses an ordinary operator save without the guard needing to know which door it was —
the clearing write being the **publish door's own head-row UPDATE**, the one that carries
`published_version += 1` (**P-D-32**: a save cannot clear it, because `inst-fd-save-txn` never
touches `published_version` and this clause requires the same statement; 06's "system save +
re-publish" names the ceremony, not the writing statement) — 06 declares the flag system-owned and never
operator-mutable, so bucket iii/iv would be the wrong home; bucket-ii columns **only while `published_version = 0` and `lifecycle_state` is non-terminal**, through `inst-fd-save-txn` (**P-D-41** names the door) (03 `inst-mt-bucket`: the draft plane
edits freely, and P-D-28's reason for bucket i applies verbatim — the whitelist named a prohibition
where the write a sibling makes legal on an unpublished head had no admitted writer), and after
first publish only through slice 07's correction act — and, as a row-image predicate this
slice pins now (**P-D-34**), **only in the same statement as a `published_version` bump**, since 07
defines its `CorrectionDoor` as "fresh-zero gate + 05 quorum + **re-publish**": the correction ends
in a publication, so it carries the same predicate `composition_pending` already does — **and, since P-D-129 (landed 2026-09-04), only where that statement also sets a `correction_ref` distinct from `OLD`'s**, the tighter predicate 07 owed and 01 paid in place, which makes door identity a physical guarantee rather than an application one (`correction_ref` itself is admitted only beside a bump, `composition_pending`'s clause repeated); **`cloned_from` never, in any
UPDATE** — stricter than bucket-i, which bites only after first publish; **bucket-i identity columns only while `published_version = 0` and `lifecycle_state`
is non-terminal, and never after first publish** (owner's call, 2026-08-27, P-D-28: the whitelist named a prohibition where every
other class named an admitting door, so the write §2 makes legal on an unpublished head had no
admitted writer; no new door — `inst-fd-save-txn` already carries the `skuCode`/`productCode`
change); and the row's update timestamp (§4.1/§4.2 `timestamps`) on every
admitted UPDATE, without which the guard would refuse every write it otherwise allows. **Row
identity — `tenant_id`, the primary key, and `created_by` — is outside the bucket scheme entirely
and admitted in no UPDATE at all** (**P-D-34**), `cloned_from`'s treatment rather than the PRD
matrix's bucket-iv catch-all, which that FR words as "other **descriptive** fields".

### 4.3 `products_entity_version` (published history)

**Key** — `(tenant_id, entity_kind, entity_id, published_version)` UNIQUE.

**What a row freezes** — the publish-time entity: frozen full content **excluding the metadata map,
and excluding `lifecycle_state`, `deprecation_provenance`, `replaced_by_sku_id` and
`internal_revision`** (**P-D-24**, owner's call, 2026-08-27, extended by **P-D-35**: those four move
on transitions, which write no version row, so freezing them would need the digest to change on a
write that produces no row to digest — they are read from the head row, below. `internal_revision`
meets the same criterion, `inst-fd-transition-bump` bumping it on **every** transition, and was left
out of the original enumeration). **`correction_ref` is excluded on a third criterion** (**P-D-129**, landed 2026-09-04): it is the correction re-publish's door identity — provenance of the version, like this row's own `approval_ref` — and not content of the entity; freezing it would make two heads with identical content digest differently. The map is slice 02's `products_metadata` (**P-D-06** — it lives
beside the entity, captured only by `CatalogVersion` snapshots). Engine-canonical serialization here
is the byte-identity discipline that `CatalogVersion` (slice 06) will reuse.

**Columns** — **a per-row content digest written at freeze**, **`content_digest`** (**P-D-35** names
it; §5's golden vector and 10's restore drill both address it), **SHA-256** over the canonical
rendering below · a **`digest_version`** column beside it, **starting at `1`** and pinned as a code
constant by §5's golden vector rather than by config (**P-D-33**) — owner's call, 2026-08-27,
P-D-29: `sha2` is already a workspace dependency, and the "digest-version bump, not a silent change"
rule below is only checkable if the version a row was computed under is stored on the row; the
slice-10 restore drill re-verifies sampled entity versions against it, and without it
version-history corruption is invisible to every checksum (H2 fix) · `approval_ref` · `actor_ref` ·
`published_at`.

**Engine-canonical serialization is pinned here** (owner's call, 2026-08-27 — 06 §2's
`inst-sn-checksum` had pointed back at "the 01 engine-canonical discipline" while this section
pointed forward, so neither stated it, and 10's restore drill re-verifies these digests byte-for-byte
against a restored copy; the cross-engine comparison is §5's golden vector):

- JSON, keys **sorted lexicographically by field name**, UTF-8 without BOM;
- **absent values written `null` rather than omitted**, so absence and the empty string cannot collide;
- integers and decimals as bare decimal strings, no locale and no trailing zeroes;
- timestamps RFC 3339 in UTC at microsecond precision;
- computed **application-side** exactly as `normalized(name)` is (§4.1), so both engines store
  identical bytes.

**A row collection inside the content** — the category-assignment set, the attribute-value set — is
rendered as a **JSON array sorted by the collection's own full row key** (the category id; the
attribute value's whole coordinate — definition, locale, region, brand), each element rendered by
the same field rule (owner's call, 2026-08-27, P-D-29: the field rule orders fields and said nothing
about rows, so two engines could have serialized the same content in two orders and 10's restore
drill compares these digests byte-for-byte).

**P-D-103 applies P-D-80 arm 1 back to these two collections**, rather than widening anything: that
decision already generalized *"by the collection's own identifier"* to **a keyed collection sorts by
its own key rendering**, and gave the manifest's entry and capture rows as its examples — it simply
never restated the rule for the two collections P-D-29 had named. Here it matters, because for one
of the two the identifier is **not unique per row**: a definition id repeats across every locale,
region and brand coordinate it carries a value at, so sorting by it orders **groups and not rows**
and leaves the within-group order to the engine — losing exactly the byte-identity this clause
exists to guarantee, on the collection that most needs it. The category-assignment set is
unaffected: its identifier *is* its row key.

**How far the rule reaches** — it is stated over **any named field set**, not only a version row's
columns (owner's call, 2026-08-27, P-D-28). **A parsed request's named field set is the fields the
request carries** (**P-D-34**): the "absent written `null`" rule addresses a *complete* set — a
version row's columns — so a `PATCH` that omits a field and one that sends it `null` hash
differently, which is what they mean at the head door, and which is what lets §3.2 hash a *parsed
request* under this same rendering so the gear pins one canonicalization rule rather than two.

**The digest's input, and what actually holds it** — the version's **frozen content** columns as
scoped above, so **adding a column to a frozen row's content is a digest-version bump, not a silent
change**. What holds all of this is a **canonical-serialization golden vector** committed with the
first migration — a different artifact from C1's schema-oracle dumps, and owed by §5.

Append-only, no UPDATE path at all; diffs are computed between rows, never stored mutated. **One
DELETE is admitted, under a referential predicate** (owner's call, 2026-08-28, **P-D-40**): a row
may be deleted only when **no `products_catalog_version_entry` references it** — 10 `inst-rt-gc`
otherwise has no admitted path, and P-D-34's repair one table over does not transfer, a version
row's collectability being a property of what still points at it rather than of the row image.
This is the first predicate here that reads another table, and it is compatible with **P-D-31**,
whose objection was to a guard reading the *door* through a session variable that exists on one
engine only: a subquery judges **data** and both engines evaluate it. It also makes 10's stated
deletion order — "entity versions only after every referencing manifest" — **physically
unbreakable** rather than procedurally promised, which is strictly more than the audit table's
window predicate buys, and it subsumes the freeze-registration arm transitively: a live
registration holds the catalog version, which holds its manifest entries, which hold these rows.
It costs 06 an index on `(tenant_id, entity_kind, entity_id, published_version)`, the manifest's
own key leading with `catalog_version_id` and being useless for this lookup.
These rows are the **only consumer-read surface** for **Product/SKU** entity *content* (08 C6's
own scoping — governed live entities are read from their live tables). **Content is not state**
(owner's call, 2026-08-27): `lifecycle_state`, `deprecation_provenance` and `replaced_by_sku_id`
move on transitions, which write no version row by design — a re-publish changes the version and
a transition changes the state, never the other way round — so those three are read from the
**head row**, which is what 08's read model already assumes in listing "identity, state + flags"
apart from content, and what lets 04 resolve `replacedBy` transitively at all: read models,
`CatalogVersion`, and the SDK's consumer-facing reads project their **content** from here — never
from head rows.
The authoring read of the head row that `inst-fd-etag`'s precondition requires is not a consumer
read.

### 4.4 `products_idempotency`, `products_audit_log`, the toolkit outbox

- `products_idempotency`: `(tenant_id, endpoint, client_key)` PK · `state` (`claimed | answered`)
  · `payload_hash` · nullable `response_status` · nullable `response_body` ·
  `expires_at`, with one CHECK tying them: `claimed` ⇒ both response columns NULL,
  `answered` ⇒ both response columns NOT NULL (§3.2; **P-D-42** removed `in_flight_until`, an
  unanswered claim no longer being able to outlive its transaction). The response columns carry a **success's** answer only (**P-D-38**: a refusal stores
  nothing and releases the key), and the replay is self-contained (P-D-29). **An `internal:` lane,
  having no wire response to reproduce, stores a synthetic `200` and its own outcome record as the
  body** (**P-D-42**): one CHECK, one shape, no nullable-for-internal arm, and absence keeps a
  single meaning in these columns. **And an `internal:` lane's `payload_hash` digests the canonical
  serialization of the act's own input record** (**P-D-69** — the bulk row's staged payload, the
  `ScheduledTransition` row, the cascade leg: one rule for the three lanes, keeping a replayed key
  with different content detectable). The cost is named rather than hidden — a status that never
  reached a wire is stored as though it had, and only a replay of an internal lane ever reads it.
- **`products_audit_log`** — `audit_id` (PK, uuid — owner's call, 2026-08-27, P-D-28: the
sealing seam's one-way UPDATE has to address a row that is not yet sealed, and `seq` is null
until it is; the surrogate is independent of the chain's ordering) · `tenant_id` · `actor_ref` (pseudonymous; the identity-reference map is slice 10's) ·
action · subject `(kind, nullable id, nullable revision)` · nullable `error_code` (the refusal's
code — §3.1 makes the code the attribution channel and AC #38 maps by it, so it is a column
rather than free text; null on the classes that are not refusals) · nullable `attempted_key`
(the natural key a pre-mint refusal carries in place of an id, §3.3's `DUPLICATE_NAME` and
`DUPLICATE_CODE` being the ordinary cases; owner's call, 2026-08-27, P-D-25) · reason (free text — it passes
`inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED`; 02
`inst-av-pii-reason` enumerates this door) · `correlation_id` (**`text`** — **P-D-118**: the W3C trace id `infra::events::correlation_id` renders, `NULL` on a background act, which has no request) · `written_at` · nullable
`session_id` · nullable `ceremony_ref` (**P-D-129**: the audit side of 07's ceremony join, the value 06's freeze ledger stores under `not_frozen(forced_at, ceremony_ref)`; written by the break-glass and correction doors, `NULL` on every other class). Three notes on the roster: `id` and `revision` are **absent on a refusal raised
before the mint**, which carries the attempted natural key (`name`, `sku_code` or `product_code`) instead — an
audit row must never name an id that identifies nothing; and `written_at` is the operand
`RetentionClock`'s audit class reads (10 `inst-rt-gc` names the class, not the column), while
`session_id` is present on the elevation
class only (05 audits every elevated access with it).

**What the table holds** — under **P-D-21**, only acts that emit no event, in three classes.
A committed mutation that *does* emit writes no row here; its outbox event is the record.

- every **refusal** with its reason — *every* refusal a registry door raises, not only the
  enumerated ones: the class is the door's, while `fr-expected-failure-behavior`'s rows are what
  12's AC #38 lint maps — three of that FR's fifteen rows sitting outside its universe so the lint is
  buildable. Scoping the audit class to that enumeration would leave `DUPLICATE_NAME`, an
  authorization denial and every `VALIDATION` rejection committing with no row;
- every **read under elevation** with its break-glass session id — elevation in v1 is
  audit-export only, so nothing under it commits a mutation and nothing under it emits;
- every **committed act the design declares emits no broker event** — **a domain act**, that is:
  one over a `Product`/`SKU` or a governed record (owner's call, 2026-08-27, P-D-27). A door's own
  infrastructure writes are not in the class, which is why this slice's
  `inst-fd-actor-ref-mint` and `inst-fd-idem-claim-write` rows declare "no event" without also
  writing an audit row — read
  literally the class would have put an audit row behind every ref resolution, a volume neither
  §4.4 nor §5 contemplates. The set was re-measured by grepping the phrase the slices actually use — five of them declare one, and the
  first two measurements of this class each missed members:
  - 04 — `PublishScheduled`/`RetirementScheduled`, "audit-plane records, explicit \"no broker
    event\" per 01 §4.5";
  - 05 — approval submissions and supersessions, "audit-plane (explicit \"no broker event\": …)";
  - 06 — freeze acks and fan-out re-triggers, "audit-plane (explicit \"no broker
    event\" — the ack door is inbound)";
  - 07 `inst-ws-no-event` — watermark ingestion, "audit-plane (explicit **no broker event** —
    watermarks arrive continuously and are queryable state, not domain history)";
  - 10 `inst-rt-gc` — GC deletes, "audit-plane, explicit **no broker event**".

  Slice 10's **erasure act** is the one exclusion, and it is the act rather than the slice: it is
  eventless only for events *carrying identity*, and 10's **Produced** set lists a minimal
  `ActorErased`. 10's GC deletes are in the class.

**How a refusal's row commits** (owner's call, 2026-08-27): in its own transaction, committing
independently of the refused mutation, and it is a **precondition of answering the caller** — if
the row cannot be written the door answers **503** and does not report the domain refusal, since
a refusal the caller learns about and the registry does not is the one thing
`nfr-availability-audit`'s "100% write-path audit" forbids. The wording this replaced had every
door write its row "in its transaction", which is precisely the transaction a refusal rolls back.
That 503 is **`AUDIT_UNAVAILABLE`** (owner's call, 2026-08-27, P-D-25) — and it is **the one
refusal the audit class carves out** (**P-D-34**; a carve-out of the audit *class*, unrelated to §3.3's
code taxonomy): its own row is by construction the one that could not be written, so the class would
otherwise carry a member it can never satisfy. It is recorded out-of-band — log and metric — and
`nfr-availability-audit`'s "100% write-path audit" is scoped to **domain** refusals.

**How a committed eventless act's row commits**: **inside the guarded mutation's transaction** —
P-D-08 S3 as amended by P-D-31 states the general rule and carves out only the refusal ("The
audit *record* stays local and commits inside the guarded mutation's transaction, as v1 already
does — **except a refusal's row, which commits in its own transaction**"). So the act and its
record stand or fall together, which is what `nfr-availability-audit`'s "100% write-path audit"
asks for on the success path.

**How a read under elevation commits**: **in its own transaction, and a precondition of serving
the read** (**P-D-34**) — S3's rule is scoped to "the guarded mutation's transaction" and a read
has none, while an elevated read the registry did not record is exactly what break-glass auditing
exists to prevent. If the row cannot be written the door answers `AUDIT_UNAVAILABLE` and serves
nothing, as a refusal does.

**Reserved platform-sealing seam (P-D-08)** — present from the first migration, never sealed by
this gear: `seal_state` (NOT NULL, roster `unsealed | sealed`) · `chain_id` · `seq` ·
`prev_hash` · `row_hash`, the last four nullable. `seal_state` is written **`unsealed` at
INSERT** by this gear, always, in v1 and after activation alike, which makes the unproven era
queryable instead of inferred from a deployment date. One CHECK ties the group so no
half-populated row exists: `unsealed` ⇒ all four NULL; `sealed` ⇒ `chain_id`/`seq`/`row_hash`
NOT NULL (a NULL `prev_hash` stays legitimate — it is the segment head). The gear computes no
hash and runs no verification job; what the platform capability must satisfy is P-D-08 S1–S9.

`products_audit_log` carries the same append-only posture as the entity tables (C5): a trigger whose whitelist admits no UPDATE or DELETE except the one below and the retention
DELETE. **The retention DELETE arm** (**P-D-34**) is a row-image predicate — a row whose
`written_at` is older than its class's retention window — so 10 `inst-rt-gc` has an admitted
path; the window's *value* is Legal/Finance's (`PRD` §15) and the predicate does not wait on it.
**The one UPDATE that whitelist admits** is what makes the seam activatable at all: a one-way
`unsealed → sealed` transition supplying `chain_id`/`seq`/`prev_hash`/`row_hash` in the same
statement, never on a row already `sealed` — a **row-image** predicate like every other guard
here (§4.2): the sealer's identity is an application and grant guarantee, not something the
trigger reads, since the session variable that would carry it exists on Postgres and not on
SQLite. Without
it, P-D-08's S3 computes the seal asynchronously **over rows already immutable**, so `row_hash`
does not exist at INSERT, the CHECK refuses an INSERT as `sealed`, and an outside-the-whitelist
column refuses the async write too — leaving exactly the migration the seam exists to avoid.
- **The outbox is the toolkit's, not this gear's** (**P-D-22**): the registry enqueues through
`toolkit_db::outbox` inside the mutation's own transaction and owns no outbox table.

- **The pipeline** is the facility's: `enqueue` → `sequencer` (per-partition sequence numbers)
  → `processor` → `vacuum`, plus dead letters — and the processor is the **broker SDK's** outbox
  producer (`gears/system/event-broker/event-broker-sdk`: a `DbProducer` bound to a
  `toolkit_db::outbox` queue, in managed **monotonic** mode), not a handler of this gear's
  (**P-D-47**). It runs in
  **`leased` (at-least-once)** mode — owner's call, 2026-08-27: a broker publish is a network
  side effect and cannot honestly join a database transaction, so `transactional` would show a
  guarantee that does not exist. The PRD does not merely tolerate the consequence:
  `fr-event-versioning-replay` requires that "out-of-order/duplicate delivery beyond the
  idempotency window **MUST** be detectable via `(tenant, aggregate, sequence)`". The envelope's
  idempotency key is the event **`id`** — minted once by the SDK at enqueue and stored in the
  outbox row, so every delivery attempt of one event carries the same value — and it is what a
  consumer dedupes on **within** the window; the `sequence` operand beyond it, after P-D-22
  superseded this slice's own `(tenant_id, aggregate_id, sequence)` index, is the **broker's**
  read-side `sequence`, server-assigned per `(topic, partition)` (**P-D-47**, re-taking the slot
  P-D-27 had named — the **Payloads** bullet below). The toolkit outbox's `seq` still orders the
  pipeline: the SDK sends it as the producer chain's `meta.sequence`, which the broker validates
  for ingest-side dedup and strips on read.
- **Ordering comes from the broker's partition selection, not from a column** (**P-D-47**): the
  gear sets no `partition_key`, so the broker's ADR-0002 default applies — MurmurHash3-32 over
  `tenant_id`, modulo `topic.partitions`, computed by the SDK for outbox routing and re-computed
  authoritatively at ingest — and every event of one tenant lands on one partition in publish
  order. That is stronger than the `(tenant, aggregate)` ordering key the envelope promises
  (`fr-registry-eventing-audit`, AC #28), and it removes the two operands this bullet could never
  pin: the hash and `N` are the broker's, and `topic.partitions` is fixed at topic creation. The
  price is stated: one partition per tenant is a per-tenant throughput ceiling the bulk lane (09)
  meets first; if it binds, `partition_key = tenant_id:aggregate_id` restores per-aggregate order
  at the cost of cross-aggregate order, and that is an amendment to P-D-47, not a tuning knob.
- **Delivery is not a state on a row.** The processor hands the message to the handler and the
  vacuum reclaims it. "Emitted" is still never reported before durable broker acceptance (PRD
  `fr-event-delivery-resilience`, registry-side half), but that is the handler's contract rather
  than a column to mark.
- **The facility brings its own multi-backend migrations**, so C1's "one migration per table"
  does not reach these tables and the schema oracle goldens them as imported.
- **Payloads**: broker-native envelope (P-D-01), and **each obligation lands where the transport
  has a slot for it** (**P-D-51**) — the versioned schema ref as the broker's `type_id`, the
  correlation id as its `trace_parent`, the ordering key as its partition selection; and **in the
  payload**, the two the broker's `Event` has no field for at all: the **causation id** and
  `actor_ref`. The idempotency key is the event `id`, which the SDK mints (**P-D-47**). Also in the
  **payload** body core (§4.5), the subject's
  `internal_revision` **as committed by the act** — N+1 where the act bumped it, the
  unchanged current value where it did not, so the number always describes the state the act left
  behind and matches the caller's next ETag (owner's call, 2026-08-27, P-D-29; "at the act" had
  admitted both readings); and, **on the envelope**, nothing of the outbox's: the slot P-D-27 named for the toolkit's
  `partition_id` and `seq` does not exist — the broker's schema marks `partition`, `sequence` and
  `sequence_time` `readOnly` and rejects them on publish — so **P-D-47** re-takes that row. The
  `(tenant, aggregate, sequence)` operand `fr-event-versioning-replay` asks for is the broker's
  own read-side `sequence`: with the tenant on one partition it is monotonic across every event
  the tenant emits, and detectability needs monotonicity, not density, so the gaps left by other
  aggregates in the same partition are harmless (P-D-27's argument for the toolkit's `seq`,
  carried to the field that exists; the `(tenant_id, aggregate_id, sequence)` index P-D-22
  superseded stays superseded) — P-D-21 makes the event the audit record of a successful act and the tuple it
  replaced named the revision. The `internal_revision` rides the **payload** body core (§4.5), not the envelope — the envelope's ordering operand is the broker's `sequence`, as 12 `inst-rc-dedup` states. The envelope is a
  platform-wide contract owned outside this gear, while the payload schema is versioned per
  event and its own rule makes an added optional field a minor bump.

### 4.5 Foundation-owned events

`ProductCreated`, `SkuCreated`, `ProductHeadSaved`, `SkuHeadSaved`, `ProductPublished`,
`SkuPublished`, `ProductDiscarded`, `SkuDiscarded`.
**Every one of the eight carries the same body core** (owner's call, 2026-08-27, P-D-27):
`{tenantId, entityKind, entityId, internalRevision, lifecycleState}` — `lifecycleState` being the
discriminator a consumer of `*HeadSaved` needs, since a save lands on a `draft`, `published` or
`deprecated` head alike. `ProductPublished`/`SkuPublished` additionally carry
**`publishedVersion`**, which is what 06 reads as content and 08's projector keys on. Anything
beyond the core is named where the act is specified, as 04 does for `SkuRetired`.
**The transition floor's three remaining
edges — `published→deprecated`, `deprecated→published`, `deprecated→retired` — carry "no event
here", and 04 announces them: `SkuDeprecated`/`ProductDeprecated` and
`SkuUndeprecated`/`ProductUndeprecated` on the first two, and — **on the SKU side only** —
`SkuRetirementEffective` on `deprecated→retired`, for which 04's Events roster names no Product
analogue and records no "no event" either (registered in 04's own open items) — `SkuRetired`/`ProductRetired` are emitted by 04 at *initiation*, not on
this edge, and re-announced by this slice's publish door during the lead window (`inst-fd-publish-reannounce`, **P-D-48**)** (owner's call,
2026-08-27; the floor stays policy-free and eventless, and 04 owns both the policy and the
announcement). Rule for every slice, **this one included** (same call — the rule had read "every
other slice", exempting the document that states it and leaving slice 12's completeness check a
blind spot on the Foundation): each
state-changing instruction names its event or records "no event" in its slice doc — **the unit
being the act, not the row** (**P-D-34**): a step inside a transaction whose event another row of
that same transaction names inherits the declaration, and only a row that is its own act owes one — the
completeness check is slice 12's. Schema versioning (`vN`→`vN+1`) discipline and the
replay/bootstrap path are specified in slice 12; the Foundation's obligation is the envelope
and the ordering key.

## 5. Testing posture (slice-local)

- Schema oracle from day one (C1): canonical dumps of both engines golden-frozen; a
  perturbation case proving the oracle can fail.
- Every refusal in §2/§3 paired with a positive control (the fixture-grants lesson); §3.4's four
  races — the `ReservationIndex` **and the `product_code` index**, the name index, the `If-Match`
  draft race, publish-vs-edit — and the claim-INSERT
  race of `inst-fd-idem-claim` all get real concurrency probes, not read-then-assert.
- The referential DELETE arm gets its own probe (**P-D-40**): deleting a `products_entity_version`
  row that a `products_catalog_version_entry` still references must be **refused by the guard**,
  not merely skipped by the GC — a probe that passes when the GC is bypassed entirely, since that
  is the case the predicate exists for.
- The trigger whitelist gets a `CorruptRow`-style probe per guarded column class (poison
  columns are the missing guards).
- A test asserting the `BucketRegistry`'s tag map and §4.2's trigger column classes name the same
  columns in the same classes — with **iii and iv asserted as one combined class**, since the
  whitelist admits them together, and **bucket-ii asserted against the interim row-image predicate §4.2 pins, re-pointed when 07
  supplies a tighter one**; the mechanical columns the whitelist names by hand (`lifecycle_state`,
  `published_version`, `internal_revision`, `deprecation_provenance`, `replaced_by_sku_id`,
  `composition_pending`, `cloned_from`, the update timestamp), **together with the row-identity
  columns `tenant_id`, the primary key and `created_by`**, carry no bucket tag and are outside the
  comparison (P-D-32 — the registry is advisory for the physical layer, so
  nothing but this test keeps the two from drifting). **A third assertion (P-D-50): no
  published-state column is named by *neither* artifact** — the case the first two are blind to by
  construction, and exactly the column the door's fail-closed miss would refuse at runtime.
- A canonical-serialization golden vector for `products_entity_version` content and its digest
  (§4.3), asserted byte-identical on both engines, pinning the `digest_version` constant (`1`) it
  was computed under.
- No `#[ignore]`d tests without a CI tier that runs them.

## 6. Traces to / Risks & Open items

**Traces to**:
- **§10 use case**: `cpt-cf-bss-products-usecase-product-sku-editor`.
- **NFRs**: #3 `cpt-cf-bss-products-nfr-publication-propagation` (the outbox half of the < 3 s
  budget; **the probe is owed and the 01/06 split is unsettled** — open in `PRD` §15 with an owner
  named), #6 `cpt-cf-bss-products-nfr-scale-extensibility` (the entity-count half: the head/version
  split and the index shape; `CatalogVersion` growth is 06's), #8
  `cpt-cf-bss-products-nfr-determinism-integrity` (the *frame* only — the pipeline, edge list and
  trigger whitelist its registered validators run in; acyclicity is 02's rule set, metering-unit
  validity 03's, the posted-period snapshot clause 06's).
- **§9 interfaces**: `cpt-cf-bss-products-interface-authoring-publish` (§9.1 — the authoring and
  publish doors, idempotency keys, `If-Match`; the id's intent-declaration clause is 06's
  `inst-rv-intent`, and 12 carries all three into the SDK) and
  `cpt-cf-bss-products-contract-registry-events` (§9.2 outbound — the broker-native envelope and
  the outbox fan-out).
- **Whole FRs**: `cpt-cf-bss-products-fr-identifier-contract`,
  `cpt-cf-bss-products-fr-idempotent-authoring`,
  `cpt-cf-bss-products-fr-skucode-reservation-concurrency`,
  `cpt-cf-bss-products-fr-registry-eventing-audit` (envelope + outbox + the audit-log arm,
  §1.6 C5/§4.4 — no sibling design document claims any clause of it).
- **FRs this slice carries a named half of**: `cpt-cf-bss-products-fr-lifecycle-transitions`
  (the machine core; the scheduling clauses are 04's),
  `cpt-cf-bss-products-fr-expected-failure-behavior` (the taxonomy's home; the retention-orphan
  row is 10's), `cpt-cf-bss-products-fr-create-product` (uniqueness; the category and
  attribute content rules are 02's), `cpt-cf-bss-products-fr-define-sku` (identity; typing
  and classification are 03's),
  `cpt-cf-bss-products-fr-revision-vs-version` (the two counters and the history; version-binding
  at freeze → 06),
  `cpt-cf-bss-products-fr-event-delivery-resilience` (registry side — durable acceptance),
  `cpt-cf-bss-products-fr-parent-child-integrity` (the interim containment check; final rule → 04),
  `cpt-cf-bss-products-fr-field-mutability-matrix` (the enforcement frame).
- **ACs**: #1, #2 (mutability frame), #5 (name uniqueness), #13, #14, #27, #28 (envelope), #38
  (frame), #42.

**Risks & open items**: eleven review passes (the numbering restarted once, at the sixth) and
the owner rounds P-D-23 through P-D-48 have run over this slice. What survives is one standing **risk** and twelve open
questions; this slice's outbound questions live at their owners.

**Risk** — a hazard rather than a question:

- this slice's interim containment check (flat subset) and slice 04 C5,
  which that slice calls "the final form of 01's interim check", must not silently diverge. The owner round leaned on exactly that relationship — it is why `SCOPE_NOT_CONTAINED` is
  declared here and not by 04 — so a change to either side that breaks it also moves the
  declaration.

**Filed elsewhere** — this slice's outbound questions live at their owners and are **not**
restated here, so nothing in this document can drift from them: `design/04-lifecycle.md`,
`design/05-governance.md`, `design/06-catalog-version.md`, `design/09-bulk-promotion.md`,
`design/12-consumer-contracts.md`, `PRD` §15, and the register.

**Open here** — **twelve**: ten raised by the eighth lens pass over the state the
P-D-35…42 rounds left, and one each by the P-D-46 and P-D-47 rounds. They are new rather than
residual, and four of the ten are consequences of those rounds:

1. ~~**Which slice declares `PARENT_NOT_PUBLISHED`?**~~ **Answered (owner, 2026-09-03), and this document had already answered itself**: §3.3 above reads *"`PARENT_NOT_PUBLISHED` (registered by slice 04 on the `→ published` target state … this code has two raising arms, and both are 04's)"*, and **P-D-97** landed the second arm as the publish door's phase continuation. **Declared here** — it is in this document's Problem-responses ladder — **and raised by 04**. `features/lifecycle.md` §7 row 18 is struck on this.
   *(Original text kept for the record:)* **Which slice declares `PARENT_NOT_PUBLISHED`?** P-D-36's unit (the raising rule) puts it with
   04, whose validator raises it; P-D-35's unit (the response map) keeps it here.
   `RETIREMENT_PENDING` sits in the identical position and resolves the other way, the only stated
   distinguisher being "unless the register moves it". 12 `inst-cc-errors` lints the pair and gets
   two answers. *(Owner: the owner of P-D-35/P-D-36.)*
2. ~~**Which code does the audit row store when a phase other than `state` collects two?**~~ **Answered (P-D-123, 2026-09-03): the refusal's own code** — `VALIDATION` for a report, each violation's code in the body — and the order of evaluation is the precedence: the run stops at the first failing phase, and inside `identity` containment is judged before the insert whose index answers `DUPLICATE_CODE`. *The item's text stood as:* §3.1 says
   only the `state` phase can, but the registered-validators phase hosts every slice's rules and
   the run stops at the first failing *phase*, and the `identity` phase hosts containment beside
   uniqueness — a create can satisfy `SCOPE_NOT_CONTAINED` and `DUPLICATE_CODE` at once. §3.3
   states a precedence for the four `state` codes only. *(Two lenses, two counterexamples. Owner: this slice with the error-contract owner.)*
3. ~~**Which phase judges an absent `If-Match`?**~~ **Answered (P-D-123, 2026-09-03): the shape phase, as `VALIDATION`** — the doors' own `OpenAPI` text says so; the precondition phase judges a mismatch, absence is a shape defect. *The item's text stood as:* §2 says it rides `VALIDATION`; §3.1 puts `If-Match`
   in the **precondition** phase and `VALIDATION` in **shape**, two phases later, while §3.1 also
   stops the run at the first failing phase. P-D-36 removed the taxonomy obstacle to either
   answer without naming one. *(Lens split: one filed it as a defect with a fix, one as a question;
   the register does not settle it. Owner: this slice.)*
4. **Is the save door's `brand_id` write validated against the caller's brand claims?** *(P-D-123, 2026-09-03: **routed to `05`, with a measurement** — the check has no operand: `SecurityContext` carries no brand claim and every door builds a tenant scope, so the create-flow clause of `inst-fd-mint-id` is inert too; beside `05` §7 row 25.)* The check is
   written into `inst-fd-mint-id` in the create flow only, while §4.2 makes the save door the sole
   admitting door for that bucket-i column while `published_version = 0`. *(Owner: this slice with
   05, who own the grants.)*
5. ~~**What happens when the SQLite busy timeout expires?**~~ **Answered (P-D-123, 2026-09-03): the platform bounds it twice** — `toolkit-db`'s `busy_timeout_ms` pragma and its 30 s `acquire_timeout`; exhaustion is a driver error, retried a bounded number of times by `transaction_with_retry` and rendered 500 with no code. No unterminated retry exists. *The item's text stood as:* §3.2 gives it no value, no exhaustion
   behaviour, no code and no status, and says two rows later that no door timeout exists anywhere in
   the set to derive one from. An unterminated retry on the dual-engine tier is the default an
   implementer builds. *(Owner: this slice.)*
6. ~~**Do the mutating doors return the new `ETag`?**~~ **Answered by the crate (P-D-123, 2026-09-03): yes, all of them** — create, the authoring `GET`, save and the head acts carry `ETAG` from `preconditions::etag(internal_revision)` on both entities. *The item's text stood as:* P-D-33's stated premise for adding the
    authoring `GET` is that an author who *had* just written holds a precondition, yet no door in §2
    is stated to return one. Leaving it makes a second `GET` mandatory between consecutive edits and
    leaves 04's and 09's in-process callers deriving the revision some other way. *(Owner: this
    slice.)*
7. **What is the `internal:` lane's stored response body?** *(P-D-123, 2026-09-03: **routed to strand C** — the body is defined by the activation runner's write, P-D-113's build.)* §4.4 has it store "a synthetic `200`
    and its own outcome record as the body" (P-D-42); `response_body` is NOT NULL on an `answered`
    row, and 05 `inst-gv-one-shot` has the `ActivationRunner` read it back after a crash. No
    document defines that record's shape for any of the three lanes. *(Owner: this slice with 04
    and 09.)*
8. ~~**When is §4.3's DELETE guard installed?**~~ **Answered (P-D-123, 2026-09-03): in `m20260829_000007`, edited in place on 2026-09-01 once `m20260901_000013` had landed the referenced table** — the chain's own convention; Postgres resolves the function body at execution and both tiers are green. *The item's text stood as:* Its predicate reads `products_catalog_version_entry`,
    which `DESIGN.md`'s census assigns to slice 06, while C1 requires one migration per table with
    guards defined once — so a trigger in this slice's first migration references a table 06 has not
    created. §5 already presumes the guard exists from the start. *(A P-D-40 consequence. Owner:
    whoever owns the migration chain.)*
9. ~~**How is `clonedFrom` physically stored?**~~ **Answered by the crate (P-D-123, 2026-09-03): two columns** — `cloned_from` (uuid, nullable) and `cloned_from_version` (nullable bigint; `NULL` under a non-null `cloned_from` means *read at the head*) on both entity tables, outside the append-only whitelist. *The item's text stood as:* 11 `inst-cn-lineage` records a pair
   `(entity id, published_version | 'draft')` while §4.1 and §4.2 provision one nullable column with
   no type — so the version half has no home, and the choice (two columns, a composite, an encoded
   text form) is load-bearing for the dual-engine rule and the append-only column whitelist.
   *(Owner: this slice, which owns the column. Filed from 11 §6, where two lenses raised it.)*

10. ~~**What refuses a request when `actor_ref` resolution itself fails?**~~ **Answered (P-D-123, 2026-09-03): a 500 with no code and no audit row** — `resolve_creator_actor_ref` renders `CanonicalError::internal`; `products_identity_ref` is the gear's own table, the 503 set is closed at three, and a refusal the gear cannot attribute is not recorded. *The item's text stood as:* §2 runs it in its own
    transaction before any phase that can refuse, and the refusal's own audit row requires an
    `actor_ref` — so an unavailable `products_identity_ref` blocks both the act and its refusal
    record, the shape the gear terminated for the audit write with `AUDIT_UNAVAILABLE` (503). No
    code, status or behaviour is stated for this one. *(Owner: this slice.)*

**And a finding about this section itself.** Until the P-D-43…49 propagation audit these outbound
questions were restated here as bullets claiming each was "registered where its owner will look".
The claim was measured twice. The eighth pass found it false for 04, 05, 09 and 12 and filed the
headline item of each; the audit found that repair itself incomplete — 05's C3 no-hook exception,
three of 12's four sub-items, and the register pointer, which had no filing mechanism at all, had
still never been filed. All five are filed at their owners now and the restatements are gone.
**A pointer is a claim like any other**: whether a §6 pointer must be verified — and
whether 12 therefore owes an open-item reciprocity lint — is itself open, for the design-set owner
with 12, as is what "the taxonomy" denotes when a count is stated against it (§3.3's enumeration,
the response map, and AC #38's rows are three different sets, and `inst-cc-errors` will be built
against a number).

11. ~~**Does the create door write content too?**~~ **Answered (P-D-123, 2026-09-03): no — the create door stays entity-only, and the clone door is the second admitted content writer**, writing inside its own transaction on the save door's terms with no save act and no revision bump, so 11's `internal_revision = 1` holds. *The item's text stood as:* **P-D-46** made `inst-fd-save-txn` the content
   writer, which settles the freeze input set for anything that has been saved. The create flow
   still writes the entity row and its outbox row and nothing else — so an entity whose content
   arrives *at creation*, which is exactly 11's clone, has no admitted writer and cannot satisfy
   11's `internal_revision = 1` if it must save afterwards. Either the create door writes content
   on the same terms, or the clone is defined as create-then-save and 11's C3 changes. Owner:
   this slice with 11's. *(Raised by the P-D-46 round — the arm's own edge.)*

12. ~~**Which GTS type does the envelope's `subject_type` name for a Product or a SKU?**~~
   **Answered by P-D-51** (owner call, 2026-08-30):
   `gts.cf.core.events.subject.v1~cf.bss.products.product.v1` and `…sku.v1`. The namespace is the
   platform's — every other subject type in this workspace is a
   `gts.cf.core.events.subject.v1~` id — and the name is this set's own declared domain type, so
   the two are traceable to each other. `PRD` §15's rule that SKUs and Products are never GTS
   *instances* is untouched: a subject type is a type. **What remains owed is not the question but
   its other half**: the event types must be registered at the broker carrying these values in
   their `allowed_subject_types`, and that registration is not this gear's to make.
