<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./02-taxonomy-attributes.md, ./06-catalog-version.md | Owners: BSS Product Catalog team -->

# DESIGN — Retention & Erasure (Slice 10)

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
  - [Erase an actor (right to erasure)](#erase-an-actor-right-to-erasure)
  - [Enforce the content-PII prohibition](#enforce-the-content-pii-prohibition)
  - [Run retention (the GC)](#run-retention-the-gc)
  - [Verify durability (the drill)](#verify-durability-the-drill)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 The identity-reference map](#31-the-identity-reference-map)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the reconciliation the PRD calls logically hardest: **immutable financial
records** (versions, snapshots, audit, events) versus **GDPR/CCPA erasure**. The resolution is
structural, not procedural: **content PII never gets in** (the write-block whose hook slice 02
hosts — this slice owns the detector policy and its allow-list), and **actor PII lives only in
the identity-reference map** (every audit row, event, and version field carries a pseudonymous
ref — erasure updates the map and touches no immutable record). Plus: retention classes to the
statutory maximum, the **retention↔grandfathering coupling** (expiry gated on the 06
freeze-registration liveness records, fail-closed), and the NFR #5 durability mechanics
(storage class, periodic checksum restore verification, DR posture).

### 1.2 Purpose

Byte-identical reproducibility and the right to erasure coexist only if erasure never has an
operand inside a frozen record. The whole gear was built that way from slice 01 (pseudonymous
`actor_ref`, `created_by`); this slice supplies the map, the erasure act, the retention clocks,
and the guards that keep a GC from orphaning a live contract.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-auditor` | Compliance reads/exports over pseudonymized trails |
| `cpt-cf-bss-products-actor-catalog-admin` | Executes erasure requests; monitors retention |
| `cpt-cf-bss-products-actor-billing` | The grandfathered-reference beneficiary of the retention gate |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.11 (`fr-retention-erasure`), §6.13
  (`fr-grandfathered-retention-coupling` — the gate half; the liveness-records half is 06's),
  §4.1 (snapshots are financial records), NFR #5; AC #35, #44; §17.1 (retention rows: statutory
  max; PII pseudonymization age)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-06 (the metadata map's **placement** — **CONFIRMED
  2026-08-26**; its PII prohibition comes from the PRD glossary / 02 C4, L4); [`./02-taxonomy-attributes.md`](./02-taxonomy-attributes.md) `inst-av-pii-block`
  (the hook this slice's policy plugs into); [`./06-catalog-version.md`](./06-catalog-version.md)
  `inst-fz-liveness` (the operand of the retention gate)

### 1.5 Scope

**In**:
- the identity-reference map + the erasure act
- the content-PII detector policy + allow-list governance (Legal)
- retention classes + clocks + the GC
- the grandfathered-retention gate
- the durability mechanics (checksum restore verification cadence, DR posture as config + probes)
- the compliance-export surface.

**Out**:
- the write-block **hook placement** (02)
- the liveness records themselves (06)
- audit row production (every slice writes its own; this slice never edits them)
- break-glass reads (05).
### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Erasure = **pseudonym-map update only**: immutable financial/version/audit/event records are never edited or deleted; because they carry only refs, updating the map completes erasure. **P-D-08 S7**: any future platform audit seal MUST exclude every field this path mutates — a seal over a resolvable identity would make erasure break the chain | PRD `fr-retention-erasure`; P-D-08 |
| C2 | Content free-text is PII-prohibited at write: hard prohibition, **fail-closed on uncertainty**, curated allow-list for legitimate person-named products, Legal sign-off recorded (PRD §15) | PRD AC #35 |
| C3 | Retention: financial/version/audit → statutory maximum (never "indefinite"); operator PII pseudonymized at erasure request or the configured max age, whichever first. **P-D-08 S8**: a platform audit seal and its anchors retain **≥** the rows they seal | PRD §17.1; P-D-08 |
| C4 | Retention expiry of a `catalogVersionId` is **gated on zero live references** in the 06 freeze-registration records; a GC that would orphan a live grandfathered reference fails closed + alerts | PRD `fr-grandfathered-retention-coupling`, AC #44 |
| C5 | Snapshots + version history: ≥ 11-nines-class replicated storage, periodic **checksum restore verification** (a restore drill that re-verifies 06 checksums, not a backup-exists check), RPO/RTO per the NFR workshop | NFR #5 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `IdentityRefMap` | `actor_ref → operator identity` — the single erasure operand |
| `PiiDetector` | The write-time content check behind 02's hook: policy + allow-list, fail-closed on uncertainty |
| `RetentionClock` | Per record class: the statutory-max schedule the GC reads |
| `RetentionGate` | The AC #44 evaluator: version-liveness from 06's records, fail-closed |

### 1.8 Context & Dependencies

**Consumed**: 02's hook (every content free-text write); 06's freeze-registration records +
checksums; config (retention durations, pseudonymization age, drill cadence); the 05 gate
(allow-list mutations are `GovernedLiveOp`s — enumerated in 05's inputs (d) as this slice's
kind). **Produced**: `ActorErased` (audit-plane semantics below), the compliance export, the
GC + its alarms, the restore-drill results surface.

## 2. Actor Flows (CDSL)

### Erase an actor (right to erasure)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-erasure`

1. [ ] - `p1` - `POST /bss-products/v1/erasure-requests` (`erasure × execute`): resolves the operator identity to its `actor_ref`s and **overwrites the map entries with tombstones** (pseudonym retained, identity gone) — one transaction, audited with a reason; no immutable record is touched (C1), and every historical read through the map now renders the tombstone - `inst-er-erase`
2. [ ] - `p1` - The act itself is recorded pseudonymously too (the eraser's own ref) — audit-plane, explicit **no broker event** carrying identity; a minimal `ActorErased(actor_ref)` broker event exists as a **defensive cache-buster**: no projection in the set materializes identities (renders join the map — M1 corrected: 08 holds pseudonyms only, and materializing an identity into any projection is a slice-12 lint failure), so the event's consumer set is legitimately empty today - `inst-er-event`
3. [ ] - `p1` - Age-based pseudonymization: the same tombstone act (emitting the same `ActorErased` — L3) runs automatically at the configured max age — **the age of the principal's last activity in the tenant** (`last_seen_at`, advanced by every act that **resolves** the principal's ref — see `inst-im-map`; age-since-first-appearance would tombstone an active employee mid-employment — M2) — erasure-on-request and erasure-on-age are one mechanism, two triggers - `inst-er-age`
4. [ ] - `p1` - **The compliance export (H3 fix)**: `GET /bss-products/v1/compliance/identity-export` (`compliance × export` — its own grant, never `audit × export`, honoring §4's exclusion): DSAR-shaped, per named principal, returning the principal's map entries + the audit-row references that carry their refs; every access individually audited. **Erasure reach (L5)**: map rows are per-tenant with one active ref per principal; a DSAR erasure enumerates the principal's rows across tenants under the platform DSAR grant, each tenant's tombstone audited in-tenant (a design statement, flagged) - `inst-er-export`

### Enforce the content-PII prohibition

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-pii-policy`

1. [ ] - `p1` - `PiiDetector` answers 02's hook: block (fail-closed, `CONTENT_PII_BLOCKED` naming the field, never the detected value) / allow / allow-by-list; **uncertainty blocks** (C2) - `inst-pp-detect`
2. [ ] - `p1` - The allow-list is a `GovernedLiveOp` under the **base approver quorum** (05 C1 — no gear-side Legal role; **P-D-10**, 2026-08-26). Legal's authority is exercised **outside** the system and enters it as a record: each entry carries a **mandatory Legal sign-off reference** (the artifact identifying the external decision) alongside its justification, and an entry offered without one is refused — which is PRD AC #35's own construction, "curated allow-list; **Legal sign-off recorded in the approval artifact**". Emits `PiiAllowlistChanged` (L3); entries are per-tenant, audited, and exportable for the Legal review. **What this deliberately does not claim:** the gear proves a Legal reference was recorded, never that Legal approved — the control is the §15 paper sign-off plus the export, and pretending otherwise would require Legal counsel to hold platform identities, which no requirement asks for - `inst-pp-allowlist`

### Run retention (the GC)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retention`

1. [ ] - `p1` - `RetentionClock` per class (the version arm reads 06's freeze-registration records, whose `released` half is **P-D-18**'s contract — a participant that never releases pins that version's storage indefinitely, which is why the release had to reach PRD §9.2) — frozen versions, catalog versions, audit, outbox-delivered, bulk ledgers, **and the evidential stores this slice owns the interplay for (M4): approval records/decisions, break-glass sessions, correction overrides (audit-grade, statutory max); watermark/member tables are operational-current state (continuously replaced, no clock needed)**: expiry candidates are computed, and for a `catalogVersionId` the `RetentionGate` requires **every freeze registration to satisfy the pair** `state = released`, **or** `state = not_frozen(forced)` **and** `released_at` stamped — never the timestamp alone (corrected 2026-08-26: an earlier repair read "`released` or carrying a `released_at`", and because nothing clears the stamp a forced participant that later recovered and acked left `state = acked` beside a live stamp, so this gate collected a version holding live grandfathered references — the very compliance event `PRD` §7 names. The pair is evaluated because a recovery moves `state` and the stale stamp then means nothing). The second arm exists because a participant that never acked cannot use the S2S release door; it releases storage and never posted-use safety) (06's release door — the H1 end-of-liveness; acked-and-unreleased = live) — a candidate with a live registration is skipped with the `retention_orphan_blocked` alarm (fail-closed: skipped, never forced; C4); GC deletes are audit-plane, explicit **no broker event** (L3) - `inst-rt-gc`
2. [ ] - `p1` - Deletion order respects reference topology (capture/entry rows before their catalog-version row; entity versions only after every referencing manifest — M3's phantom "counter history" removed); every GC act is audited with the class, the clock, and the gate verdict - `inst-rt-order`
3. [ ] - `p1` - Entity-version rows referenced by ANY retained `CatalogVersion` manifest are retained with it (p1 — the only rule stopping the GC from orphaning a manifest, M3) (the manifest's entity half references frozen rows — 06 H3): version-row retention derives from catalog-version retention, never shorter - `inst-rt-derive`

### Verify durability (the drill)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-restore-drill`

1. [ ] - `p2` - On the configured cadence: restore a sampled set of catalog versions **and their referenced entity versions** from backup into an isolated target and re-verify **both** the 06 manifest checksums and the per-row entity-version digests (01 §4.3 — H2 fix: manifest checksums alone are blind to version-history corruption) byte-for-byte (C5) — a mismatch is a compliance incident alarm, not a log line; results land on an operator surface with the last-verified watermark per tenant - `inst-rd-drill`

## 3. Processes / Business Logic

### 3.1 The identity-reference map

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-identity-map`

1. [ ] - `p1` - `products_identity_ref` — `(tenant_id, actor_ref)` → identity payload | tombstone, `first_seen_at`, **`last_seen_at`** (the M2 age operand, **advanced by every act that resolves the ref — not by minting it**: minting happens exactly once on first appearance and there is one active ref per `(tenant, principal)`, so "refreshed by every ref-minting act" left the column pinned to `first_seen_at` forever and age-based erasure tombstoned an active employee mid-employment — the precise failure M2 records as fixed, item 23 of the 2026-08-26 review. Every door that stamps an `actor_ref` onto an audit row, an approval, a decision, a session or an override resolves the ref and therefore advances it; the write is a same-transaction touch, not a separate act); one active ref per `(tenant, principal)` (L5) — **and a tombstoned ref is retired permanently**
  (2026-08-26, PR #14 review): erasure tombstones the map entry while every append-only record
  keeps the `actor_ref` it was stamped with, so re-minting that key for the same principal later
  would make render-time joins show the **new** identity against historical rows. A principal
  acting after its erasure therefore mints a **fresh** ref; the Foundation minting path enforces
  it, and "first appearance" means first appearance of a principal *with no live ref*; written on first appearance of a principal (01's doors mint refs through it); **the only table in the gear where PII may live** — which holds only because **every operator free-text field is inside the content-PII write block** (item 24 of the 2026-08-26 review: 02's block covered attribute/description free text and the metadata map, while mandatory or optional `reason` fields exist on audit rows, approval rejections, break-glass sessions, correction overrides and the `SkuRetired` broker payload — personal data typed there was unreachable by erasure, since these records are never edited and erasure is a map-only tombstone, C1). The detector policy and allow-list are this slice's and unchanged; what widens is the **set of doors that invoke them**: every `reason`-bearing door, enumerated, raising the same `CONTENT_PII_BLOCKED`. Fail-closed at the door is the only reach erasure can have over a record it may not rewrite, and the only one erasure writes - `inst-im-map`
2. [ ] - `p1` - Reads join through the map at render time (08 projections, audit exports, approval queues) — no surface caches resolved identities beyond its own rebuildable projections - `inst-im-render`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-contract-retention-errors`

`ERASURE_UNKNOWN_ACTOR` (the erasure door, when the named principal resolves to no `actor_ref` in this tenant — named 2026-08-26). (`CONTENT_PII_BLOCKED` is **declared in 02's taxonomy** — the door is
02's, only the verdict policy lives here; kept out of this owned list per the one-declaration
rule, L1.) The GC and drill raise alarms, not API errors.

**Problem responses (RFC 9457):** `ERASURE_UNKNOWN_ACTOR`, `CONTENT_PII_BLOCKED` (422).

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
all, **404** only where a path segment names a resource this tenant has none of, **503** where retry
is the remedy. **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the
platform rule: no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for a **canonical** error in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `CONTENT_PII_BLOCKED` (slice 02) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's map (the PII exception table — excluded from every export except the compliance
surface, encrypted at rest per platform posture); `products_pii_allowlist` (governed entries +
justifications); retention/drill state is config + audit, no new record tables. Events per
§2 (the deliberately minimal `ActorErased`).

## 5. Testing posture (slice-local)

- **The reproducibility-vs-erasure flagship**: freeze a version → erase its approver → the old
  snapshot's checksum is unchanged AND the rendered audit shows the tombstone (both halves in
  one probe — C1 is only proven by asserting both).
- Detector matrix: block / allow / allow-by-list / uncertainty-blocks, each with a positive
  control; the allow-list mutation runs the base quorum (05 C1) **and** is refused when the
  Legal sign-off reference is absent — asserted with its positive control, since a
  mandatory-field rule proven only by its refusal is a rule that may never admit anything.
- Retention gate RED: a candidate version with one live freeze-registration is skipped +
  alarmed; the same version GCs cleanly once the registration ends (the AC #44 pair).
- Derived retention: an entity-version row referenced only by a retained catalog version
  survives its own class clock.
- Restore drill: a deliberately corrupted backup sample fails the drill loudly (the oracle must
  be seen to fail — the perturbation discipline).
- Age-based pseudonymization fires without a request and is byte-identical in effect to the
  requested path.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-retention-erasure` (clocks, the erasure act and the retention gate; the content write-block is slice 02's), `cpt-cf-bss-products-fr-grandfathered-retention-coupling` (gate
half), `cpt-cf-bss-products-fr-expected-failure-behavior` (the "retention process that would orphan a live
grandfathered reference" row — `retention_orphan_blocked`, L2), AC #35, #38 (that row), #44;
**NFR #5 `cpt-cf-bss-products-nfr-snapshot-archival-dr` (the restore-drill and archival mechanics, and the cold re-resolution MUST of `PRD` §7's row; durability mechanics are shared with slice 06)**; §17.1 retention rows; C2's Legal sign-off (§15 open — the design is ready
either way).

**Risks & open items**:

- **OPEN (2026-08-26, PR #14 review) — the cross-tenant DSAR reach has no grant and conflicts
  with the isolation constraint.** `inst-er-export`'s L5 clause has a DSAR erasure enumerate the
  principal's rows **across tenants** "under the platform DSAR grant". No such grant exists in
  05's RBAC catalog, which asserts every door names its pair; and a tombstone is a write, while
  05 C5 says "any write under elevation is refused, full stop" and `DESIGN.md`'s
  `constraint-tenant-isolation` limits v1 break-glass to read/audit-export. Two ways out and
  they are not equivalent: define a platform-plane `compliance × erase` grant outside the
  tenant elevation path, or make v1 erasure per-tenant and state that a principal spanning
  tenants needs one request per tenant. **Owner: Architecture + Legal.** Until it is decided,
  `inst-er-erase`'s "erasure completes" claim holds only within one tenant.
- **Detector quality is a product risk, not a design one**: fail-closed-on-uncertainty
  guarantees safety and guarantees friction; the allow-list loop must exist before GA (the 02
  risk restated as this slice's operational owner), and the §15 Legal sign-off covers the
  posture itself.
- **Watermark/member tables (07) and bulk ledgers** carry `skuId`s and row payloads, not PII —
  asserted here so their retention rides ordinary classes; if a future producer's payload grew
  identity-bearing fields, the map discipline would apply — named to keep it from drifting in
  silently.
- **Encrypted-at-rest for the map** rides the platform storage posture; if a deployment lacks
  it, this table is the one that must not ship — a deployment gate, not a code path.
