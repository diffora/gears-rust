<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./04-lifecycle.md, ./05-governance.md | Owners: BSS Product Catalog team -->

# DESIGN — CatalogVersion & Freeze (Slice 6)

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
  - [Request → increment (the D-47 lanes)](#request--increment-the-d-47-lanes)
  - [Build the snapshot](#build-the-snapshot)
  - [Resolve a version (declared intent)](#resolve-a-version-declared-intent)
  - [The freeze protocol](#the-freeze-protocol)
  - [Grandfathering invariant](#grandfathering-invariant)
  - [`compositionPending` clearing](#compositionpending-clearing)
  - [Diff two versions (AC #20a)](#diff-two-versions-ac-20a)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Concurrency & ordering](#31-concurrency--ordering)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
  - [3.3 Observability (the posting-safe budget)](#33-observability-the-posting-safe-budget)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the registry's reproducibility anchor: the **`CatalogVersion` machine**
(demand-driven, mechanical increments over the D-47 lanes; per-tenant serialization; gapless
monotonic ids), the **full-snapshot builder** with canonical serialization and checksum
(byte-identical re-resolution forever), the **freeze protocol** (`freezeComplete`, acks,
bounded fail-closed timeout, recovery + force-completion, governed participant set snapshotted
per version), the **grandfathering invariant**, the `compositionPending` clearing lane, the
**catalog-version diff** (AC #20a), and the **version-binding-at-freeze** clause of
`fr-revision-vs-version` assigned here by the design set's traceability.

### 1.2 Purpose

Posted invoices and contracts must resolve the same bytes in five years that they froze today
(`pricingSnapshotRef`'s `catalogVersion` segment is this slice's product). Everything here is
**mechanical** (P-D-02): governance happened at entity publish; an increment snapshots
already-governed content within a ratified SLO and never waits on a human.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Operator-initiated publish; freeze monitoring, re-trigger, force-completion |
| `cpt-cf-bss-products-actor-plan-price` | Requests addressability (D-47 pending-ref), owes `freezeComplete` acks + the composition signal |
| `cpt-cf-bss-products-actor-contracts` / `…-actor-billing` | Freeze participants (both currently silent counterparts — PRD §15) |
| `cpt-cf-bss-products-actor-subscriptions` | Grandfathering beneficiary: frozen snapshots never move |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.6 (all six FRs + `fr-catalog-version-diff`), §6.13
  (`fr-catalog-publish-concurrency`, `fr-grandfathered-retention-coupling` — liveness source
  half), §7 (posting-safe/propagation/archival NFRs); AC #19–#25, #20a, #40, #44
- [`../DECISIONS.md`](../DECISIONS.md) P-D-02 (mechanical increments), P-D-06 (metadata
  capture); pricing D-47 (lanes + SLO — the joint contract), pricing `design/01-foundation.md`
  §4.4 (`pricing_catalog_version_ref` pending→committed, `commit_overdue`)
- PRD §15 opens consumed here: freeze-ack counterparts silent; `BundleCompositionCompleted`
  unregistered in pricing Slice 8

### 1.5 Scope

**In**: increment lanes + coalescing + serialization + gapless ids; snapshot builder (content
manifest, canonical serialization, checksum, P-D-06 metadata capture, participant-set
snapshot); resolution API with declared intent; freeze protocol end-to-end (acks, timeout,
recovery, force-completion, participant governance); grandfathering invariant + per-version
freeze-registration records (the AC #44 liveness source slice 10 consumes); `compositionPending`
clearing; the diff; the posting-safe observability.

**Out**: entity publish and its governance (01/05); what participants do with the fan-out
(their gears); retention/GC execution (10 — this slice only supplies the liveness records);
`pricingSnapshotRef` composition (rating); the pricing-side pending-ref table (pricing owns it).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | D-47 lanes, ratified: interactive request → increment with coalescing ≤ 5 s; bulk → one version, hard max delay 5 min; pending→committed p95 ≤ 60 s / max 5 min; the registry is the **sole** incrementer | pricing D-47 / PRD `fr-catalog-version-publish` |
| C2 | An increment is mechanical — never an approval gate; the CatalogVersion-publish lint is informational for operator publishes | P-D-02 |
| C3 | A published `CatalogVersion` is immutable and non-withdrawable (roll-forward N+1 only); snapshot boundary = the whole tenant, serialized | PRD `fr-catalog-version-publish` |
| C4 | Re-resolving a `catalogVersionId` yields a byte-identical checksum at any future time; the snapshot stores **references** to plan-price/Contracts/Billing content, never that content | PRD `fr-snapshot-reproducibility` |
| C5 | Resolution for posted/contractual use is refused until `freezeComplete`; the timeout fails closed; browse may proceed; the caller **declares intent** | PRD `fr-freeze-atomicity` |
| C6 | Snapshots are financial records: durability/DR posture per NFR #5 (storage class + periodic checksum restore verification — the mechanics land with slice 10's retention posture) | PRD §4.1, NFR #5 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `IncrementRequest` | A queued demand for addressability — **the whole contract, stated here once**: `(source, lane ∈ {interactive, bulk}, request_key, operation_key?, requested_at)`, **UNIQUE on `(source, request_key)`** (the idempotency and `satisfiedRequests` operand) with `operation_key` naming the bulk operation whose requests coalesce into one version. Pricing's pending-ref request or an operator publish. **The contract is the `products-sdk` increment-request client, not a transport** (manifest §3.3.2/§9.1: contracts are transport-agnostic, in-process `ClientHub` composition is the default mode, `runtime.type: local \| oop` switches without code changes); §4's REST door is its out-of-process binding and its authz door |
| `SnapshotBuilder` | Collects the tenant's published set + live captures into a manifest, serializes canonically, checksums |
| `VersionManifest` | The snapshot's content: entity-version references + captured live content (categories, definitions, category values, metadata maps) + participant-set snapshot |
| `FreezeLedger` | Per-version ack state: participant → `acked \| pending \| not_frozen(forced)` |
| `IntentfulResolver` | The resolution API: `(catalogVersionId, intent ∈ {browse, posted})` |

### 1.8 Context & Dependencies

**Consumed**: `SkuPublished`/`ProductPublished` (01 fan-out — what is publishable content);
pricing's addressability requests (D-47) and — once pricing registers it — the composition
signal; the slice-05 gate only for **operator ceremonies** (force-completion two-person,
participant-set governance); config (coalescing windows, freeze timeout — also the floor of
01's idempotency retention C6). **Produced**: `CatalogVersionPublished` (with changed-entity
list), `FreezeForceCompleted`, `SkuCompositionCleared` (**outbound** — the inbound plan-price signal keeps the name `BundleCompositionCompleted`, §1.4); the committed version ids pricing
finalizes its pending refs against; the freeze-registration records slice 10 gates retention
on; the diff surface.

## 2. Actor Flows (CDSL)

### Request → increment (the D-47 lanes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-increment`

1. [ ] - `p1` - An `IncrementRequest` arrives through the **`products-sdk` increment-request client** — a typed contract the consumer resolves from `ClientHub`, never an implementation package (manifest §3.4.1); the default deployment binds it **in-process**, and `POST /bss-products/v1/catalog-version-requests` (S2S; `catalog_version × request`) is the same contract's out-of-process binding and the authz door both bindings pass. The transport is not the performance axis — the lane SLO below is p95 ≤ 60 s, against which a round trip is noise — it is the **error axis**: an in-process binding cannot fail with a network error, which is why the client's error taxonomy separates "not wired" from "unreachable" from "unusable answer" the way pricing's own `ProductCatalogClientV1` already does for the opposite direction. The request carries `{source, lane, request_key, operation_key?}` — idempotent per `(source, request_key)`; a **bulk** request names its `operation_key` so the whole operation coalesces into ONE version (M3 fix). The trigger set (M1 fix): registered downstream addressability requests (pricing; **and this gear's own slice-09 bulk commits — a registered internal requester** whose ledger completion sends the `close` marker on its `(source, operation_key)`), and the operator **catalog-publish act** — an entity publish NEVER enqueues an increment (the PRD §6.6 preamble is substance, not a 5-second technicality; a retirement's `effectiveAt` flip likewise does not enqueue — the next demand-driven version reflects it, L5 intended) - `inst-cv-request`
2. [ ] - `p1` - The **coalescer** (one worker per tenant — C3 serialization) drains the queue: interactive requests coalesce within ≤ 5 s of the earliest pending; a **keyed bulk batch stays open** until its operation closes (completion signal or the **5-minute hard max** from its earliest request) and lands as ONE version — interactive versions may publish in between without shredding it (M3 fix: D-47's "bulk coalesces into one version" holds per `operation_key`, not per quiet window) - `inst-cv-coalesce`
3. [ ] - `p1` - The increment transaction: allocate the next `catalog_version_id` from the per-tenant counter row (gapless by construction — the counter update and the version insert share the transaction), build the manifest (flow below), commit, emit `CatalogVersionPublished` carrying the changed-entity list vs the previous version **and `satisfiedRequests` — the `(source, request_key)` set this version committed** (H1 fix: pricing's finalizer maps its pending refs by its own request keys; a pure-pricing batch has an empty changed-entity list but never an empty `satisfiedRequests`). **No approval is consulted** (C2) - `inst-cv-commit`
4. [ ] - `p1` - SLO instrumentation: `requested_at → published_at` p95 ≤ 60 s / max 5 min; a pending request past the lane deadline raises `catalog_version_overdue` (the registry-side mirror of pricing's `commit_overdue`) - `inst-cv-slo`

### Build the snapshot

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-snapshot`

1. [ ] - `p1` - `SnapshotBuilder` collects, inside the serialized transaction: every published/deprecated entity's **current published version reference** (into `products_entity_version` — 01's only-consumer-read-surface rule makes this the sole content source), current categories + their live display values (02 H2 branch), current attribute definitions, per-entity **metadata maps as of this instant** (P-D-06), the recognized sets, and the **freeze-participant set snapshot** (AC #23). **Every live capture is a STORED COPY** (canonical serialization into the manifest's capture store — H3 fix): category values, metadata, recognized sets and the participant set have no frozen versions of their own, so a reference to their live rows would break byte-identity the moment they moved; only the Product/SKU halves are references (into the immutable `products_entity_version` rows) - `inst-sn-collect`
2. [ ] - `p1` - **Stage-vs-commit re-validation (AC #40)**: the builder records each collected entity's `(id, published_version, lifecycle_state)`; before commit it re-reads the heads — any entity whose published version **or lifecycle state** moved between collect and commit (the AC's explicit "mutated or retired" arm — H2 fix) **fails the run closed naming the entity** (`STAGED_ENTITY_CHANGED` for an operator publish; a mechanical run re-coalesces and retries fresh, the request never lost — **the lane split is now normative: `fr-catalog-publish-concurrency` and AC #40 were amended 2026-08-26 to state it (P-D-09), so this is no longer a design reading of the word "rejected"**) - `inst-sn-revalidate`
3. [ ] - `p1` - Canonical serialization + checksum over the manifest (the 01 engine-canonical discipline); references to sibling-gear content only (C4). Byte-identity probe: a re-resolution renders from the stored manifest, never re-collects - `inst-sn-checksum`
4. [ ] - `p1` - **Version binding at freeze** (`fr-revision-vs-version`, third clause — assigned here): when a bound-not-yet-frozen reference re-resolves to a newer version, the **resolve/finalize response itself carries `(boundVersion, resolvedVersion, diffRef)`** — the diff is surfaced TO the module, not left for it to know to pull (M4 fix); the `CatalogVersionPublished` changed-entity list and the AC #20a diff surface back it for arbitrary spans, and the module's duty to act on the diff is a seam-suite assertion (slice 12) - `inst-sn-binding-diff`

### Resolve a version (declared intent)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-resolve`

1. [ ] - `p1` - `IntentfulResolver` (`catalog_version × read` — the shared version-lookup component behind resolve AND diff is the single raising door of `CATALOG_VERSION_UNKNOWN`, L3) requires `intent`: absent ⇒ `INTENT_REQUIRED` (400-class, the consumer-side obligation the seam suite will assert); `browse` serves any published version at once; `posted` is refused `FREEZE_INCOMPLETE` until the `FreezeLedger` reads complete (C5) - `inst-rv-intent`
2. [ ] - `p1` - Re-resolution is byte-identical forever: content renders from the stored manifest + frozen entity versions; the checksum is returned and verifiable - `inst-rv-bytes`

### The freeze protocol

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-freeze`

1. [ ] - `p1` - On `CatalogVersionPublished`, every participant in the **version's snapshotted set** owes an ack; `products_freeze_ack` records them; `freezeComplete` = all acked (AC #21). The ack door (`catalog_version × ack`) accepts a participant's ack **only from that participant's own service identity** (S2S claims — the registration check `PARTICIPANT_UNKNOWN` is membership, not authn); idempotent per `(version, participant)` - `inst-fz-ack`
2. [ ] - `p1` - The bounded timeout (configured; its value floors 01's idempotency retention) fails **closed**: past it the version stays non-posting-safe and `freeze_overdue` alarms name the silent participants (today: all three — the PRD §15 open is visible in this gear's own telemetry from day one) - `inst-fz-timeout`
3. [ ] - `p1` - Recovery: the fan-out **re-trigger is idempotent** (same event, same version, at-least-once safe); **force-completion** is a slice-05 two-person ceremony (`catalog_version × force_complete`; refusals ride the 05 gate's own codes — L4: no separate `FORCE_COMPLETE_QUORUM`) recording each missing participant as `not_frozen(forced)` and flipping the version to **`freezeComplete = complete(forced)`**: posted resolution now succeeds, and the response carries the **per-participant frozen state** — refusing the `not_frozen` participant's content is the **consumer's** seam obligation (slice 12; the snapshot holds only references to that content, C4, so no registry door can refuse it — M5 fix). `FreezeForceCompleted` emitted (AC #22) - `inst-fz-force`
4. [ ] - `p1` - Participant-set membership is a `GovernedLiveOp` (`freeze_participant × write`; material — slice 05 input (d), where 06's kinds are enumerated — M6 fix); each change emits `FreezeParticipantSetChanged` (participants must learn they were added); each version resolves `freezeComplete` against **its own snapshotted set** forever: removal after publish never retro-flips a historical version (AC #23) - `inst-fz-membership`
5. [ ] - `p1` - **Freeze-registration records are the version-liveness source** (AC #44): per `(catalogVersionId, participant)` registration/ack rows persist as the operand slice 10's retention gate reads — never the per-SKU reference count, which carries no version dimension. **Liveness ends by an explicit release** (H1 fix of the slice-10 review): the `catalog_version × release` door (S2S, the participant's own identity) records that the participant holds no more live references to that version (its contracts expired, subscriptions closed); the ledger state becomes `released`, and version-liveness = acked-and-not-yet-released. Until participants deliver releases the gate stays conservative — correct, and now with a designed exit instead of a vacuous forever - `inst-fz-liveness`

### Grandfathering invariant

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-grandfathering`

1. [ ] - `p1` - A frozen snapshot referenced by a grandfathered consumer is **never mutated** — held by construction: manifests and entity versions are append-only (01 C5), and retirement/deprecation touch head rows only (04). This instruction exists so the delegation is auditable from the registry side: eligibility policy is plan-price/subscriptions-lifecycle's, the immutability is ours - `inst-gf-invariant`

### `compositionPending` clearing

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-composition-clear`

1. [ ] - `p2` - The inbound composition signal (pricing's — **unregistered on their side, PRD §15**; the registry-side door is designed now so their adoption is one event handler) names the composed `bundle` SKU; the registry clears `composition_pending` as a **system save + re-publish** of the head (version N+1) — **not exempt from the gate**: it runs as a `system_signal` approval subject auto-satisfied by the **signal itself as the authorizing principal** (recorded on the `ApprovalRecord` with the signal reference — the approver is the governed pricing-side act, named and audited, rather than an exemption). The satisfaction is **independent of the tenant's configured `N`** (05 C1, P-D-11): a `system_signal` subject neither consumes the human quorum nor is exempt from the gate, because `N` governs human approvals of **operator** acts and the governance for this one already happened pricing-side. *(This clause previously leaned on 05's "nothing publishes approver-less" interim, which P-D-11 retired when the count gained floor 0.)* The flag stays system-owned, never operator-mutable, emits `SkuCompositionCleared` — **this gear's outbound event, distinct from the inbound `BundleCompositionCompleted` that drove it** (one name had carried both directions until 2026-08-26; a registry emitting the very event it consumes is a loop, not a contract) — and audits with the signal reference. Prior frozen versions keep the flag as it was (C4) - `inst-cc-clear`

### Diff two versions (AC #20a)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-diff`

1. [ ] - `p2` - `GET …/catalog-versions/{a}/diff/{b}` covers **every snapshot member**: entities added/removed + per-entity published-version deltas (rendering the 01 history diff), **and the capture half — category tree and display values, attribute definitions, recognized sets, per-entity metadata maps, the participant/producer sets** (a metadata-only or live-entity-only change between two versions must appear; the manifest's own membership is the diff's universe) — computed read-only from the two stored manifests, byte-stable for a given pair, no retention effect (AC #20a) - `inst-df-diff`

## 3. Processes / Business Logic

### 3.1 Concurrency & ordering

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-cv-concurrency`

1. [ ] - `p1` - One increment worker per tenant (advisory lock / queue partition): publishes serialize (AC #40), the counter row makes ids gapless and monotonic; entity publishes are **not** blocked by a running increment — they land on heads, and the re-validation step decides whether the run must retry - `inst-cvc-serial`
2. [ ] - `p2` - The changed-entity list in `CatalogVersionPublished` is computed against the immediately previous version inside the same transaction — fan-out ordering per tenant is the version order by construction - `inst-cvc-order`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-cv-errors`

`INTENT_REQUIRED`, `FREEZE_INCOMPLETE`, `STAGED_ENTITY_CHANGED` (operator publish lane),
`CATALOG_VERSION_UNKNOWN` (one door: the shared version-lookup — L3), `PARTICIPANT_UNKNOWN`
(ack from an unregistered participant — refused, audited). Ceremony refusals ride the 05
gate's own codes (L4).

### 3.3 Observability (the posting-safe budget)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-algo-posting-safe`

NFR #4's program SLO decomposes here: `commit → event durably accepted` (01 outbox metric),
`event → ack` per participant (this ledger), `requested → published` (C1). Gauges:
pending-request age per lane, unacked participants per version, `freeze_overdue` /
`catalog_version_overdue` alarms; the posting-safe composite is derivable from these three
without a fourth clock.

## 4. Data / Storage (normative shape; DDL in migrations)

- **`products_catalog_version`** — `(tenant_id, catalog_version_id)` PK (monotonic per tenant)
  · `checksum` · `staged_at` / `published_at` · `participant_set_snapshot` · `freeze_state`
  (derived cache of the ledger) · manifest header. Append-only, physically guarded.
- **`products_catalog_version_entry`** — `(tenant_id, catalog_version_id, entity_kind,
  entity_id)` → `published_version` (references into immutable `products_entity_version`); plus
  the **capture store** rows — `(tenant_id, catalog_version_id, capture_kind)` → the **stored
  canonical copy** of category values / metadata maps / recognized sets / freeze-participant set
  / reference-producer set (07's symmetric-snapshot ride) as-of the snapshot (H3 fix: live
  content is copied, never referenced). The manifest body;
  append-only; the checksum covers both halves.
- **`products_catalog_version_counter`** — `(tenant_id)` → next id (the gapless allocator).
- **`products_catalog_version_request`** — the queue: `source`, `lane`, **`request_key`**
  (UNIQUE with `source` — the idempotency and `satisfiedRequests` operand), **`operation_key`**
  (nullable; the bulk batch identity), **`closed_at`** (the retry-safe close marker — a repeated
  close is a no-op), `requested_at`, state `(pending, coalesced-into(version), superseded)`.
- **`products_freeze_participant`** — the governed registered set (live);
  **`products_freeze_ack`** — `(tenant_id, catalog_version_id, participant)` → `state ∈
  {pending, acked, released, not_frozen(forced)}` with `acked_at` / **`released_at`** /
  not_frozen(forced_at, ceremony_ref)`; together the `FreezeLedger` and the AC #44 liveness
  records (never GC'd while their version exists — slice 10 contract).
- **Events**: `CatalogVersionPublished` (changed-entity list, `satisfiedRequests`, checksum,
  participant set), `FreezeForceCompleted`, `FreezeParticipantSetChanged`,
  `SkuCompositionCleared`; acks and re-triggers are audit-plane (explicit "no broker
  event" — the ack door is inbound).

## 5. Testing posture (slice-local)

- **Byte-identity flagship**: publish → mutate everything mutable (heads, metadata, categories,
  recognized sets) → re-resolve the old version → checksum unchanged (extends the 02 P-D-06
  probe to the full manifest).
- Gapless/serialized probe: concurrent increment requests on one tenant → sequential ids, no
  gaps, one worker (real concurrency, not read-then-assert).
- AC #40 probes, both arms: entity **re-published** AND entity **retired/deprecated** between
  collect and commit → operator lane fails naming the entity; mechanical lane retries fresh and
  the request survives (H2: the lifecycle arm is the one a version-only check misses).
- Freeze: timeout → posted-intent resolution refused; force-completion records `not_frozen` and
  posted use of that participant's content stays refused; historical version re-resolves
  `freezeComplete` against its snapshotted set after a membership change (AC #23 probe).
- Lane SLOs under a bulk burst: one version, ≤ 5-min delay, interactive deadline honored in a
  mixed window.
- Composition clear: prior frozen version keeps `compositionPending = true`; the new version
  reads false; the clear survives replay (idempotent per signal reference).

## 6. Traces to / Risks & Open items

**Traces to (PRD)**: `fr-catalog-version-publish`, `fr-snapshot-reproducibility`,
`fr-freeze-atomicity`, `fr-freeze-recovery`, `fr-freeze-participant-governance`,
`fr-grandfathering-invariant`, `fr-bundle-adoption-guard` (registry half),
`fr-catalog-version-diff`, `fr-catalog-publish-concurrency`,
`fr-grandfathered-retention-coupling` (liveness-source half; retention gate → 10),
`fr-revision-vs-version` (version-binding-at-freeze clause); AC #19–#25, #20a, #40, #44
(records half); NFR #3/#4/#5 (budgets; durability mechanics with 10), NFR #6
(`CatalogVersion`-growth half: the capture-store economics + publishes/day target, L2).

**Risks & open items**:
- **The composition-clear publish is RESOLVED (2026-08-26), and it is not an exemption**: the
  2026-08-26 CodeRabbit pass forced the question, and the honest shape turned out to be an
  approval *subject* rather than a carve-out — 05 `inst-gv-*` subject kind `system_signal`,
  whose `ApprovalRecord` is auto-satisfied with the **inbound governed signal as the authorizing
  principal**, audited like any decision, with no human approver and **no exemption from the
  gate**. `DESIGN.md`'s status line dropped it from the human-flag list in the same wave; this
  bullet had been left behind saying the opposite.
- **All three freeze participants are §15-silent**: the protocol ships registry-complete with
  `freeze_overdue` naming them from day one; until at least pricing registers, every version is
  posting-unsafe by construction — correct, loud, and worth a product decision on v1 launch
  sequencing (freeze participants before first posted use).
- **Full-snapshot economics** (NFR #6): entry-per-entity manifests are O(catalog) per version;
  the §15/NFR-workshop publishes-per-day target bounds storage — the manifest table is designed
  for dedup later (the entity half references immutable version rows; the capture half stores
  copies — H3 — and is the part a delta-encoding would compress; a compatible optimization,
  named to keep it out of v1).
- **Bulk-lane starvation**: a steady interactive trickle must not defer a bulk window past its
  5-min hard max — the coalescer's deadline logic gets a probe when built.
