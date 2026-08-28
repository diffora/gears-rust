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
  §4.1 (snapshots are financial records), NFR #5, `fr-expected-failure-behavior` (the retention-orphan row); AC #35, #38, #44; §17.1 (retention rows: statutory
  max; PII pseudonymization age)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-06 (the metadata map's **placement** — **CONFIRMED**; its PII prohibition comes from the PRD glossary / 02 C4, L4); [`./02-taxonomy-attributes.md`](./02-taxonomy-attributes.md) `inst-av-pii-block`
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
- audit row **editing** (every slice writes its own; this slice never edits them)
- break-glass reads (05).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Erasure = **pseudonym-map update only**: immutable financial/version/audit/event records are never edited, and never deleted by erasure; because they carry only refs, updating the map completes erasure. **P-D-08 S7**: any future platform audit seal MUST exclude every field this path mutates — a seal over a resolvable identity would make erasure break the chain | PRD `fr-retention-erasure`; P-D-08 |
| C2 | Content free-text is PII-prohibited at write: hard prohibition, **fail-closed on uncertainty**, curated allow-list for legitimate person-named products, Legal sign-off recorded (PRD §15) | PRD AC #35 |
| C3 | Retention: financial/version/audit → statutory maximum (never "indefinite"); operator PII pseudonymized at erasure request or the configured max age, whichever first. **P-D-08 S8**: a platform audit seal and its anchors retain **≥** the rows they seal | PRD §17.1; P-D-08 |
| C4 | Retention expiry of a `catalogVersionId` is **gated on zero live references** in the 06 freeze-registration records; a GC that would orphan a live grandfathered reference fails closed + alerts | PRD `fr-grandfathered-retention-coupling`, AC #44 |
| C5 | Snapshots + version history: ≥ 11-nines-class replicated storage, periodic **checksum restore verification** (a restore drill that re-verifies 06 checksums, not a backup-exists check), RPO/RTO per the NFR workshop | NFR #5 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `IdentityRefMap` | `(tenant_id, actor_ref) → operator identity` (`products_identity_ref`, §4) — the single erasure operand |
| `PiiDetector` | The write-time content check behind 02's hook: policy + allow-list, fail-closed on uncertainty |
| `RetentionClock` | Per record class: the statutory-max schedule the GC reads |
| `RetentionGate` | The AC #44 evaluator: version-liveness from 06's records, fail-closed |

### 1.8 Context & Dependencies

**Consumed**: 02's hook (every content free-text write); 06's freeze-registration records +
checksums; config (retention durations, pseudonymization age, drill cadence); the 05 gate
(allow-list mutations are `GovernedLiveOp`s — enumerated in 05's inputs (d) as this slice's
kind). **Produced**: `ActorErased` (audit-plane semantics below), `PiiAllowlistChanged`, the compliance
export, the GC + its alarms, the restore-drill results surface.

## 2. Actor Flows (CDSL)

### Erase an actor (right to erasure)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-erasure`

1. [ ] - `p1` - `POST /bss-products/v1/erasure-requests` (`erasure × execute`): resolves the operator identity to its `actor_ref`s and **overwrites the map entries with tombstones** (pseudonym retained, identity gone) — one transaction, audited with a reason; no immutable record is touched (C1), and every historical read through the map now renders the tombstone - `inst-er-erase`
2. [ ] - `p1` - The act itself is recorded pseudonymously too (the eraser's own ref) — audit-plane, explicit **no broker event** carrying identity; a minimal `ActorErased(actor_ref)` broker event exists as a **defensive cache-buster**: no projection in the set materializes identities (renders join the map — M1 corrected: 08 holds pseudonyms only, and materializing an identity into any projection is a slice-12 lint failure), so the event's consumer set is legitimately empty today - `inst-er-event`
3. [ ] - `p1` - Age-based pseudonymization: the same tombstone act (emitting the same `ActorErased` — L3) runs automatically at the configured max age — **the age of the principal's last activity in the tenant** (`last_seen_at`, advanced by every door that **stamps** the principal's `actor_ref` onto a record — see `inst-im-map`; age-since-first-appearance would tombstone an active employee mid-employment — M2) — erasure-on-request and erasure-on-age are one mechanism, two triggers - `inst-er-age`
4. [ ] - `p1` - **The compliance export (H3 fix)**: `GET /bss-products/v1/compliance/identity-export` (`compliance × export` — its own grant, never `audit × export`, honoring §4's exclusion): DSAR-shaped, per named principal, returning the principal's map entries + the audit-row references that carry their refs; every access individually audited. **Erasure reach (L5)**: map rows are per-tenant with one active ref per principal; a DSAR erasure enumerates the principal's rows across tenants under the platform DSAR grant, each tenant's tombstone audited in-tenant (a design statement, flagged) - `inst-er-export`

### Enforce the content-PII prohibition

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-pii-policy`

1. [ ] - `p1` - `PiiDetector` answers 02's hook: block (fail-closed, `CONTENT_PII_BLOCKED` naming the field, never the detected value) / allow / allow-by-list; **uncertainty blocks** (C2) - `inst-pp-detect`
2. [ ] - `p1` - The allow-list is a `GovernedLiveOp` (`pii_allowlist × write`) under the **base approver quorum** (05 C1 — no gear-side Legal role; **P-D-10**). Legal's authority is exercised **outside** the system and enters it as a record: each entry carries a **mandatory Legal sign-off reference** (the artifact identifying the external decision) alongside its justification, and an entry offered without one is refused — which is PRD AC #35's own construction, "curated allow-list; **Legal sign-off recorded in the approval artifact**". Emits `PiiAllowlistChanged` (L3); entries are per-tenant, audited, and exportable for the Legal review. **What this deliberately does not claim:** the gear proves a Legal reference was recorded, never that Legal approved — the control is the §15 paper sign-off plus the export, and pretending otherwise would require Legal counsel to hold platform identities, which no requirement asks for - `inst-pp-allowlist`

### Run retention (the GC)

*Volume note (**P-D-27**): 01 §4.4's third audit class — committed acts that emit no broker event —
covers **domain** acts only, over a `Product`/`SKU` or a governed record. A door's own
infrastructure writes (an `actor_ref` resolution, an idempotency claim) are outside it, so this
slice's audit class does not grow with them.*

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retention`

1. [ ] - `p1` - `RetentionClock` per class (the version arm reads 06's freeze-registration records, whose `released` half is **P-D-18**'s contract — a participant that never releases pins that version's storage indefinitely, which is why the release had to reach PRD §9.2) — frozen versions, catalog versions, audit — whose rows carry `audit_id` (PK, uuid — **P-D-28**, the address the sealing seam's one-way UPDATE needs) and, on a refusal, nullable `error_code` and `attempted_key` (**P-D-25**), so a retention sweep of the audit class deletes whole rows and never leaves a refusal without its classifier — bulk ledgers (outbox-delivered rows are the **toolkit vacuum's** horizon, not a class this clock
computes candidates for — **P-D-22**; this slice owed that correction), **and the evidential stores this slice owns the interplay for (M4): approval records/decisions, break-glass sessions, correction overrides (audit-grade, statutory max); watermark/member tables are operational-current state (continuously replaced, no clock needed)**: expiry candidates are computed, and for a `catalogVersionId` the `RetentionGate` requires **every freeze registration to satisfy the pair** `state = released`, **or** `state = not_frozen(forced)` **and** `released_at` stamped — never the timestamp alone (corrected: an earlier repair read "`released` or carrying a `released_at`", and because nothing clears the stamp a forced participant that later recovered and acked left `state = acked` beside a live stamp, so this gate collected a version holding live grandfathered references — the very compliance event `PRD` §7 names. The pair is evaluated because a recovery moves `state` and the stale stamp then means nothing). The second arm exists because a participant that never acked cannot use the S2S release door; it releases storage and never posted-use safety (06's release door — the H1 end-of-liveness; acked-and-unreleased = live) — a candidate with a live registration is skipped with the `retention_orphan_blocked` alarm (fail-closed: skipped, never forced; C4); GC deletes are audit-plane, explicit **no broker event** (L3) - `inst-rt-gc`
2. [ ] - `p1` - Deletion order respects reference topology (capture/entry rows before their catalog-version row; entity versions only after every referencing manifest — **and the last of those is physically enforced, not merely ordered**: 01 admits the entity-version DELETE only when no manifest entry references the row, 01 **P-D-40** — M3's phantom "counter history" removed); every GC act is audited with the class, the clock, and the gate verdict - `inst-rt-order`
3. [ ] - `p1` - Entity-version rows referenced by ANY retained `CatalogVersion` manifest are retained with it (p1 — with `inst-rt-order`'s DELETE precondition, what stops the GC from orphaning a manifest, M3) (the manifest's entity half references frozen rows — 06 H3): version-row retention derives from catalog-version retention, never shorter - `inst-rt-derive`

### Verify durability (the drill)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-restore-drill`

1. [ ] - `p2` - On the configured cadence: restore a sampled set of catalog versions **and their referenced entity versions** from backup into an isolated target and re-verify **both** the 06 manifest checksums and the per-row entity-version `content_digest`
(01 §4.3, named by **P-D-35**, which lists this instruction as a propagation target) (01 §4.3 — H2 fix: manifest checksums alone are blind to version-history corruption) byte-for-byte (C5). **The digest is SHA-256 over 01 §4.3's canonical rendering, and each frozen row carries the `digest_version` it was computed under (P-D-29)** — the drill compares like with like, and re-verifying a row written under an earlier digest version is a version mismatch rather than a corruption alarm. **The frozen column set excludes `lifecycle_state`, `deprecation_provenance`, `replaced_by_sku_id` and `internal_revision` (P-D-24, extended by P-D-35)**: those move on transitions that write no version row, so a drill must not expect them in the digested content — a mismatch is a compliance incident alarm, not a log line; results land on an operator surface with the last-verified watermark per tenant - `inst-rd-drill`

## 3. Processes / Business Logic

### 3.1 The identity-reference map

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-identity-map`

1. [ ] - `p1` - `products_identity_ref` — `(tenant_id, actor_ref)` → identity payload | tombstone, `first_seen_at`, **`last_seen_at`** (the M2 age operand, **advanced by every act that resolves the ref — not by minting it**: minting happens once per active ref — on the first appearance of a principal with no live ref and there is one active ref per `(tenant, principal)`, so "refreshed by every ref-minting act" left the column pinned to `first_seen_at` forever and age-based erasure tombstoned an active employee mid-employment — the precise failure M2 records as fixed, item 23 of the review. Every door that stamps an `actor_ref` onto an audit row, an approval, a decision, a session or an override resolves the ref and therefore advances it; the write is a same-transaction touch, not a separate act); one active ref per `(tenant, principal)` (L5) — **and a tombstoned ref is retired permanently**
  (PR #14 review): erasure tombstones the map entry while every append-only record
  keeps the `actor_ref` it was stamped with, so re-minting that key for the same principal later
  would make render-time joins show the **new** identity against historical rows. A principal
  acting after its erasure therefore mints a **fresh** ref; the Foundation minting path enforces
  it, and "first appearance" means first appearance of a principal *with no live ref*; written on first appearance of a principal (01's doors mint refs through it, **in the mint's own
transaction ahead of the guarded operation — P-D-26**, so a first-time principal whose opening act
is *refused* still has a committed ref for the refusal's audit row to attribute to; a ref minted
for a refused act is normal and is exactly what `last_seen_at` should record); **the only table in the gear where PII may live** — which holds only because **every operator free-text field is inside the content-PII write block** (item 24 of the review: 02's block covered attribute/description free text and the metadata map, while mandatory or optional `reason` fields exist on audit rows, approval rejections, break-glass sessions, correction overrides, bulk/promotion row reasons and the `SkuRetired` broker payload (02's canonical enumeration) — personal data typed there was unreachable by erasure, since these records are never edited and erasure is a map-only tombstone, C1). The detector policy and allow-list are this slice's and unchanged; what widens is the **set of doors that invoke them**: every `reason`-bearing door, enumerated, raising the same `CONTENT_PII_BLOCKED`. Fail-closed at the door is the only reach erasure can have over a record it may not rewrite, and the only one erasure writes - `inst-im-map`
2. [ ] - `p1` - Reads join through the map at render time (08 projections, approval queues — never `audit × export` output, §4) — no surface caches resolved identities - `inst-im-render`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-retention-errors`

`ERASURE_UNKNOWN_ACTOR` (the erasure door, when the named principal resolves to no `actor_ref` in this tenant — named). (`CONTENT_PII_BLOCKED` is **declared in 02's taxonomy** — the door is
02's, only the verdict policy lives here; kept out of this owned list per the one-declaration
rule, L1.) The GC and drill raise alarms, not API errors.

**Problem responses (RFC 9457):** `ERASURE_UNKNOWN_ACTOR`, `CONTENT_PII_BLOCKED` (422).

*Statuses added, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `CONTENT_PII_BLOCKED` (slice 02) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's map (the PII exception table — excluded from every export except the compliance
surface, encrypted at rest per platform posture); `products_pii_allowlist` (governed entries + justifications + the mandatory Legal sign-off
reference); retention/drill state is config + audit, no new record tables. Events per
§2 (the deliberately minimal `ActorErased`, and `PiiAllowlistChanged`).

## 5. Testing posture (slice-local)

- **The reproducibility-vs-erasure flagship**: freeze a version → erase its approver → the old
  snapshot's checksum is unchanged AND the rendered audit shows the tombstone (both halves in
  one probe — C1 is only proven by asserting both).
- Detector matrix: block / allow / allow-by-list / uncertainty-blocks, each with a positive
  control; the allow-list mutation runs the base quorum (05 C1) **and** is refused when the
  Legal sign-off reference is absent — asserted with its positive control, since a
  mandatory-field rule proven only by its refusal is a rule that may never admit anything.
- Retention gate RED: a candidate version with one live freeze-registration is skipped +
  alarmed; the same version GCs cleanly once **every** registration satisfies the pair (`state = released`,
  or `state = not_frozen(forced)` with `released_at` stamped); a registration at `state = acked`
  beside a stamped `released_at` is still skipped + alarmed (the AC #44 pair, and the correction's own regression).
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

- **OPEN (PR #14 review) — the cross-tenant DSAR reach has no grant and conflicts
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
- **Which store holds the audit rows this slice's own rules require?** `inst-er-erase` is "audited
  with a reason", `inst-er-export` audits "every access individually", and 01 §4.4 holds — under
  **P-D-21** — "only acts that emit no event, in three classes". The erasure act emits
  `ActorErased`, so it writes no row there; the compliance export is a read that is not "a read
  under elevation"; and the minimal `ActorErased(actor_ref)` carries neither the reason nor the
  eraser's own ref. Either 01 §4.4 gains a class, `ActorErased` widens, or these acts are declared
  eventless. Owner: 01's owner with P-D-21's. *(Two lenses raised it independently — slice-10 first lens pass.)*
- **What `actor_ref` attributes an unattended act's audit row?** The age-triggered tombstone and
  every GC act are audited, 01 makes the audit row's `actor_ref` non-nullable, and every ref in
  the set is minted for a human principal on first appearance. No document names a system ref or
  admits a null one. Owner: 01's owner with this slice. *(Raised by the slice-10 first lens pass.)*
- **How is the map read by principal?** §4 states one key, `(tenant_id, actor_ref)`, while three
  rules read it the other way — erasure resolves an operator identity *to* its refs, the DSAR
  export is "per named principal", and "first appearance" is defined as a principal with no live
  ref. No document names a principal column, an index on it, or the shape of "identity payload" —
  and the payload is exactly what a tombstone destroys, so a uniqueness rule sited in it cannot
  survive erasure. Owner: this slice, with 01 as the consumer of the resolve path. *(Raised by the slice-10 first lens pass.)*
- **Does the retention gate collect a version that has no freeze registrations at all?** The gate
  is universally quantified over registrations, so an empty set satisfies it vacuously, while C4
  says a GC that would orphan a live grandfathered reference fails closed. This is the same
  vacuity 06 §6 already registers for the ledger's creation point, over the same rows. 06 stores
  `participant_set_snapshot` per version, so a non-vacuous domain exists; no document says the
  gate ranges over it. Owner: 06's owner with this slice. *(Raised by the slice-10 first lens pass.)*
- **Where does a successful restore drill's state live?** `inst-rd-drill` puts "the last-verified
  watermark per tenant" on an operator surface; §4 says "retention/drill state is config + audit,
  no new record tables"; and a per-tenant watermark is neither config nor an append-only audit
  row. `DECISIONS.md` already records this as owed under P-D-21 and deliberately unapplied.
  Owner: this slice with P-D-21's owner. *(All three lenses raised it independently — slice-10 first lens pass.)*
- **What does the drill do on a digest-version mismatch, and what is the corruption alarm
  called?** The mismatch arm is named and not terminated — nothing says whether the row is
  skipped, re-rendered under its stored version, or reported — the alarm is unnamed where every
  other alarm in the set has a name, and the sample rule ("a sampled set") is unstated. 01 pins
  `digest_version` starting at `1` as a code constant, so no second version yet exists to test the
  arm against. Owner: this slice with the NFR #5 workshop. *(Raised by the slice-10 first lens pass.)*
- **Which surfaces may resolve an identity through the map, and under what grant?**
  `compliance × export` is "its own grant, never `audit × export`", and it is the only grant any
  document attaches to a map read — yet `inst-im-render` has approval queues and 08 projections
  resolving at render time. 08 states it renders "actor pseudonyms" and never mentions the map or
  a join, so the two slices disagree about what 08 does. 12 forbids *storing* an identity
  elsewhere and says nothing about resolving one. Owner: 05's RBAC catalog owner with this slice
  and 08. *(Two lenses raised it independently — slice-10 first lens pass.)*
- **Who owns NFR #5's cold re-resolution MUST, and how does the clause split with 06?** Both
  slices claim the requirement as "shared", while 12 requires exactly one owner per clause. The
  word "cold" appears nowhere in the design set except this slice's Traces-to line: there is no
  instruction, no §4 shape and no §5 probe for cold resolution or its p95, and 06 — which owns the
  resolver — claims only the durability half. Owner: the design-set owner with 06.
  *(Two lenses raised it independently — slice-10 first lens pass.)*
- **Who delivers the DR half of the durability mechanics?** §1.1 promises "storage class, periodic
  checksum restore verification, DR posture" and §1.5 "checksum restore verification cadence, DR
  posture as config + probes", and the only body
  rule is the drill; storage class and RPO/RTO appear only as a constraint deferred to the NFR
  workshop. Either an instruction owes the DR config and probes, or Scope In owes a narrowing.
  Owner: the design owner, with the NFR workshop's output in hand. *(Raised by the slice-10 first lens pass.)*
- **Which actor holds `compliance × export`?** The door returns real identities, the only
  compliance actor in §1.3 is described as reading "over pseudonymized trails", and 05's RBAC
  catalog has no such pair for this gear. Entangled with the cross-tenant DSAR item already open
  here. Owner: Architecture with Legal. *(Raised by the slice-10 first lens pass.)*
- **Is `products_pii_allowlist` itself a PII store?** By construction it holds person-named
  strings, and its entries are "exportable for the Legal review", yet only the map carries the
  posture "excluded from every export except the compliance surface, encrypted at rest". The same
  question decides whether the allow-list's justification and sign-off fields belong in §3.1's
  content-PII write block — this pass synced that enumeration to 02's canonical list and
  deliberately did **not** add these two fields. Owner: Legal with the data-protection owner. *(Raised by the slice-10 first lens pass.)*
- **What code does the allow-list's missing-sign-off refusal carry?** `inst-pp-allowlist` refuses
  an entry offered without a Legal sign-off reference, §5 asserts that refusal with a positive
  control, and §3.2 declares no code for it — so the door answers unclassified and the SDK error
  enum has no member for a refusal a caller will routinely hit. Either it rides 01's `VALIDATION`
  or this slice declares its own. Owner: this slice with the error-contract owner. *(Raised by the slice-10 first lens pass.)*
- **What does "byte-identical in effect" mean for the age-triggered path?** §5 asserts the age
  path is byte-identical in effect to the requested path, while the requested path is "audited
  with a reason" and the age path has no requester and no supplied reason. Nothing says whether
  the audit row is part of "effect" or only the map state. Owner: this slice. *(Raised by the slice-10 first lens pass.)*
