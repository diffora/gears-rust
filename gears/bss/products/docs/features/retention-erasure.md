# Feature: Retention & Erasure

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-retention-erasure-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-retention-erasure`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Erase an actor](#erase-an-actor)
  - [Enforce the content-PII prohibition](#enforce-the-content-pii-prohibition)
  - [Run retention](#run-retention)
  - [Verify durability](#verify-durability)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [The identity-reference map](#the-identity-reference-map)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The identity-reference map's act, over a store that already ships](#the-identity-reference-maps-act-over-a-store-that-already-ships)
  - [The erasure door](#the-erasure-door)
  - [Age-based pseudonymization, as the same act](#age-based-pseudonymization-as-the-same-act)
  - [The compliance export](#the-compliance-export)
  - [The PII detector policy](#the-pii-detector-policy)
  - [The Legal-governed allow-list](#the-legal-governed-allow-list)
  - [The retention clocks](#the-retention-clocks)
  - [The retention gate, and the pair it evaluates](#the-retention-gate-and-the-pair-it-evaluates)
  - [Deletion order, and the one edge that is physically enforced](#deletion-order-and-the-one-edge-that-is-physically-enforced)
  - [The restore drill](#the-restore-drill)
  - [The error taxonomy](#the-error-taxonomy)
  - [The authz surface, and the four rosters it reddens](#the-authz-surface-and-the-four-rosters-it-reddens)
  - [The two events, and the one that is deliberately minimal](#the-two-events-and-the-one-that-is-deliberately-minimal)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/10` §6](#carried-verbatim-from-design10-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

The reconciliation that is logically hardest: **immutable financial records** — versions,
snapshots, audit rows, events — against **GDPR/CCPA erasure**. The resolution is **structural, not
procedural**: content PII never gets in, and actor PII lives only in the identity-reference map, so
erasure updates the map and touches no immutable record. This feature owns that map and the erasure
act, the content-PII detector policy and its Legal-governed allow-list, retention classes and their
clocks, the retention↔grandfathering gate, the compliance-export surface, and the durability
mechanics.

### 1.2 Purpose

Byte-identical reproducibility and the right to erasure coexist **only if erasure never has an
operand inside a frozen record**. The gear was built that way from `01-foundation` — every audit
row, event and version field carries a pseudonymous `actor_ref` — and this feature supplies the map
that ref resolves through, the act that tombstones it, the clocks, and the guards that keep a
garbage collector from orphaning a live contract.

**Requirements**: `cpt-cf-bss-products-fr-retention-erasure`,
`cpt-cf-bss-products-fr-grandfathered-retention-coupling`,
`cpt-cf-bss-products-fr-expected-failure-behavior`,
`cpt-cf-bss-products-nfr-snapshot-archival-dr`

**Principles**: `cpt-cf-bss-products-principle-fail-closed`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: **none** — DECOMPOSITION §2.10 states it: erasure and the GC are background acts over
the tables the other features write.

**Divided requirements, and which half is this feature's.** All four are shared, and the division is
the design set's:

| requirement | this feature's half | the other half |
|---|---|---|
| `fr-retention-erasure` | the clocks, the erasure act and the retention gate | the content write-block **hook**, which is `02-taxonomy-attributes`' — only the detector *policy* is here |
| `fr-grandfathered-retention-coupling` | the retention gate | the liveness records it reads (`06-catalog-version`) |
| `fr-expected-failure-behavior` | the retention-orphan row and its `retention_orphan_blocked` alarm | the taxonomy's home (`01-foundation`) |
| `nfr-snapshot-archival-dr` | restore verification and the DR posture | the archival and snapshot operand (`06-catalog-version`) |

**Out of scope**: the write-block **hook placement** (`02-taxonomy-attributes`); the liveness records
themselves (`06-catalog-version`); audit-row **editing**, which no slice does and this one least of
all; and break-glass reads (`05-governance`).

**Not applicable**: no state machine is declared here — see §4.

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Executes erasure requests; monitors retention and the drill |
| `cpt-cf-bss-products-actor-auditor` | Compliance reads and exports over pseudonymized trails |
| `cpt-cf-bss-products-actor-billing` | The grandfathered-reference beneficiary of the retention gate — it never calls a door here; the gate exists so its references are not orphaned |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md) — §6.11 (`fr-retention-erasure`), §6.13
  (`fr-grandfathered-retention-coupling`, the gate half), §4.1 (snapshots are financial records),
  NFR #5, `fr-expected-failure-behavior`'s retention-orphan row; AC #35, #38, #44; §17.1 (retention
  rows at statutory max; the PII pseudonymization age)
- **Design**: [DESIGN.md](../DESIGN.md); the granular module boundary is
  [`../design/10-retention-erasure.md`](../design/10-retention-erasure.md) (313 lines)
- **Decisions**: [DECISIONS.md](../DECISIONS.md) — **P-D-06** (the metadata map's placement),
  **P-D-08** (S7's seal exclusion and S8's seal retention), **P-D-10** (no gear-side Legal role),
  **P-D-18** (the release door), **P-D-21** (the audit classes), **P-D-22** (the toolkit vacuum's
  horizon), **P-D-24**/**P-D-35**
  (the frozen column set), **P-D-25**, **P-D-26** (the ref mint's own transaction), **P-D-27**
  (the audit class's scope), **P-D-28**, **P-D-29** (`digest_version` on the row), **P-D-49**
  (`principal_ref`, and the gate over `participant_set_snapshot`), **P-D-40** (the referential
  `DELETE` predicate), **P-D-50** (per-tenant erasure, and the strike of 09 from the reason-door
  enumeration)
- **Dependencies**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-catalog-version`

**The declaration site, and what this document may define.** `design/10`'s four `flow-` and one
`algo-` declarations **moved here**; each of its sections now carries a pointer at this file and
keeps its own instruction steps, which stay normative. One definition site per id.

This document defines only `flow`, `algo`, `dod` and `featstatus` ids. **It declares no `inst-`
id** — §4 mints none because this feature has no state machine (see §4).

`design/10` §3.2 carried only `cpt-cf-bss-products-contract-retention-errors`, so
**`cpt-cf-bss-products-algo-retention-errors` is minted here** and §3.2 points at it — the **eighth**
`contract-`-only §3 section in the set to have needed this.

**This document does not copy the slice's instruction steps.** §5 does restate storage shapes,
because a DoD must name the columns it obliges — **`design/10` §3.1 and §4 govern on any
column-level fact**. Bare `(C1)`–`(C5)` throughout this document are **`design/10` §1.6's**
constraint rows; a foreign gear's constraint is always named with its slice, as at §2's
`05-governance` C5.

**The four foreign instruction ids this feature reaches, and all four are written.**

| id | slice | FEATURE written? |
|---|---|---|
| `inst-av-pii-block` | `02-taxonomy-attributes` | yes — the hook this feature's policy plugs into |
| `inst-fz-liveness` | `06-catalog-version` | yes — the operand of the retention gate |
| `inst-gv-liveop-gate` | `05-governance` | yes — the gate the allow-list mutation passes |
| `inst-mt-inputs` | `05-governance` | yes — where this feature's `GovernedLiveOp` kind is registered material |

**Four is the lowest foreign-seam count of any capability feature written so far** — `06`'s was
five, `07`'s nine and `05-governance`'s twelve — and all four land in written FEATUREs.

**Positive findings against the shipped crate.**

Byte-verified at `80eee534a`. This feature's shape differs from the last two in **where** the crate's attention sits, not in
whether it has any. `06`'s five and `07`'s twelve were each a count of **one** thing — the
migration naming the referential predicate, and the Foundation naming the correction door — over
greenfield stores. Here the crate names this slice **31 times across 15 files**, and `RetentionClock`
four times, while **no door of this feature exists** and its central operand already ships, built
for erasure. Two of those namings are obligations this feature must discharge in someone else's
file: `m20260829_000004`'s owed retention arm and `m20260829_000007`'s owed `DELETE` predicate.

- **`products_identity_ref` ships with the tombstone model already in the schema.** The migration
  and `entity::identity_ref` carry it: `identity_payload` is **nullable** because *"a tombstone
  destroys it while `principal_ref` — the pseudonym, not the identity — stands, which is what lets a
  repeat DSAR and the age predicate keep working after an erasure"*;
  `chk_products_identity_ref_tombstone` **requires the payload absent** once `tombstoned_at` is set;
  `tombstoned_at` is *"Set once, by erasure, and never cleared"*; and
  `uq_products_identity_ref_active` is a **partial** unique index on `(tenant_id, principal_ref)
  WHERE tombstoned_at IS NULL` — the physical form of the one-active-ref rule, and what makes *"first
  appearance"* mean first appearance of a principal **with no live ref**. The table deliberately
  carries **no append-only guard**: *"`last_seen_at` and the tombstone columns are mutable by
  design"*. **So the DoDs below oblige the act, not the store.**
- **`last_seen_at` already carries the corrected semantics.** Its own doc says it is *"advanced by
  every act that **resolves** the ref, never by minting it alone"* — which is exactly the M2
  correction `inst-im-map` records, and the reason age-based erasure does not tombstone an active
  employee mid-employment.
- **The restore drill's seam is agreed on both sides.** The crate names *"slice 10's restore drill"*
  in **fourteen** places, two of them stating properties it depends on: `repo.rs` calls one *"the single
  property slice 10's restore drill depends on"*, and `m20260829_000007` explains that a row could be
  *"perfectly intact and still fail slice 10's restore drill"* if the storage type re-rendered its
  input — which is why `content` is `text` on both engines rather than `jsonb`. And
  `canonical::DIGEST_VERSION`'s doc gives this drill as the reason the version is stored on the row:
  *"Storing it on the row is what lets slice 10's restore drill re-verify a sampled entity version
  against the rule it was actually computed under; without it, version-history corruption is
  invisible to every checksum."* `inst-rd-drill` says the drill compares like with like. **Neither
  side has to be persuaded of the other's obligation.**
- **The DELETE this feature's GC spends is already built.** *(Re-measured 2026-09-03; at
  `80eee534a` it was specified and unbuilt, which is what §7 row 17 recorded before its close.)*
  `06-catalog-version`'s `cpt-cf-bss-products-dod-referential-delete-predicate` is **ticked**, and
  `m20260829_000007`'s unconditional refusal has been replaced in place by **P-D-40**'s referential
  predicate on both engines. That predicate is what `inst-rt-order` calls *"physically enforced, not
  merely ordered"*. **06 specified the guard and shipped it; this feature owns the deleter.**

## 2. Actor Flows (CDSL)

Each flow below is **declared here and stepped in
[`../design/10-retention-erasure.md`](../design/10-retention-erasure.md) §2**, whose steps are the
normative ones. What this section carries is the triggering actor, the scenarios and the boundary.

### Erase an actor

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-erasure`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin` for the requested path; **the age clock
(system)** for the automatic trigger

**Success Scenarios**:
- The door resolves the operator identity to its `actor_ref`s and **overwrites the map entries with
  tombstones** — pseudonym retained, identity gone — in **one transaction**, audited with a reason.
  **No immutable record is touched** (C1), and every historical read through the map now renders the
  tombstone.
- The act is recorded **pseudonymously too**, under the eraser's own ref.
- **Age-based pseudonymization is the same act on a second trigger.** At the configured maximum age
  — the age of the principal's **last activity** in the tenant, not of its first appearance — the
  same tombstone runs automatically and emits the same event. Erasure-on-request and erasure-on-age
  are one mechanism with two triggers.
- The **compliance export** returns, per named principal, that principal's map entries plus the
  audit-row references carrying their refs, and **every access is individually audited**.

**Error Scenarios**:
- A named principal resolving to no `actor_ref` in this tenant is `ERASURE_UNKNOWN_ACTOR`, **naming
  the principal**.

**Boundary, and the reach this feature deliberately does not claim.** Erasure is **per-tenant in
v1** (**P-D-50**): a DSAR enumerates and tombstones the principal's rows **in the requesting tenant
only**, and a principal appearing in several tenants needs one request per tenant. **No
platform-plane DSAR grant is minted**, because that alternative creates a write path outside tenant
elevation, which `constraint-tenant-isolation` and `05-governance` C5 both forbid — and the gear
does not build one on an assumption about what Legal requires. Should Legal rule per-tenant erasure
incomplete, **the platform grant becomes mandatory and is a post-v1 change rather than a gap in this
rule**.

### Enforce the content-PII prohibition

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-pii-policy`

**Actor**: every operator writing free text, through `02-taxonomy-attributes`' hook

**Success Scenarios**:
- The detector answers the hook with **block / allow / allow-by-list**, and **uncertainty blocks**
  (C2).
- An allow-list entry is a `GovernedLiveOp` under the **base approver quorum** — there is no
  gear-side Legal role (**P-D-10**) — carrying a **mandatory Legal sign-off reference** beside its
  justification, and emitting `PiiAllowlistChanged`. Entries are per-tenant, audited and exportable
  for the Legal review.

**Error Scenarios**:
- A block is `CONTENT_PII_BLOCKED`, **naming the field and never the detected value**.
- An allow-list entry offered without a Legal sign-off reference is refused — see §7 for the code it
  carries, which the design set does not state.

**Boundary, stated because the honest limit is narrower than the control sounds.** The gear proves a
Legal reference **was recorded**, never that Legal approved. The control is the paper sign-off plus
the export; claiming more would require Legal counsel to hold platform identities, which no
requirement asks for. And the **hook placement** is `02-taxonomy-attributes`' — only the policy and
the allow-list are this feature's.

### Run retention

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retention`

**Actor**: the GC (system), monitored by `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- A clock per record class computes expiry candidates: frozen versions, catalog versions, audit
  rows, and the **evidential stores this feature owns the interplay for** — approval records and
  decisions, break-glass sessions, correction overrides, all audit-grade at statutory maximum.
- For a `catalogVersionId` the gate ranges over **that version's `participant_set_snapshot`**
  (**P-D-49**), not over whatever ledger rows happen to exist, and requires **every** freeze
  registration to satisfy the **pair**: `state = released`, **or** `state = not_frozen(forced)`
  **and** `released_at` stamped.
- Deletion order respects reference topology, and the last edge is **physically enforced rather than
  merely ordered**: an entity-version row is deletable only when no manifest entry references it
  (**P-D-40**).
- An entity-version row referenced by **any** retained catalog-version manifest is retained with it:
  version-row retention derives from catalog-version retention and is **never shorter**.
- Every GC act is audited with the class, the clock and the gate verdict.

**Error Scenarios**: none — the GC raises **alarms, not API errors**.
- A candidate with a live registration is **skipped** with `retention_orphan_blocked`. Fail-closed:
  skipped, never forced (C4).

**And the gate does not fire in v1.** `design/06`'s `inst-fz-liveness` measures it: the v1 registered
set is pricing alone, which is §15-silent, so **every** version's registration sits `pending` and the
AC #44 collection gate never fires. That is the fail-safe direction — the gate over-retains, never
over-collects — but the catalog-version arm must be read as *designed and not yet exercised* rather
than as a working reclamation path.

**Boundary, and two things the clock deliberately does not own.** Outbox-delivered rows are the
**toolkit vacuum's** horizon (**P-D-22**), not a class this clock computes candidates for; and the
watermark and member tables of `07-reference-signal` are **operational current state**, continuously
replaced, needing no clock. The liveness records the gate reads are `06-catalog-version`'s.

**Why the predicate is a pair and not a timestamp.** An earlier reading — *"`released` or carrying a
`released_at`"* — collected a version holding live grandfathered references, because nothing clears
the stamp: a forced participant that later recovered and acked left `state = acked` beside a live
stamp. The pair is evaluated because a recovery **moves `state`** and the stale stamp then means
nothing. An **empty** snapshot — a tenant with no participant registered at publish — has nobody who
ever owed an ack and is collectable; quantifying over registrations instead let an empty ledger
satisfy the gate vacuously.

### Verify durability

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-restore-drill`

**Actor**: the drill (system), on the configured cadence

**Success Scenarios**:
- A sampled set of catalog versions **and their referenced entity versions** is restored from backup
  into an **isolated target**, and **both** the manifest checksums and the per-row
  `content_digest` are re-verified **byte-for-byte** (C5).
- Results land on an operator surface with the **last-verified watermark per tenant**.

**Error Scenarios**: none at a door — the drill raises alarms.
- A byte mismatch is a **compliance incident alarm**, not a log line.
- A row written under an **earlier `digest_version`** is a **version mismatch**, not a corruption
  alarm: the drill compares like with like, which is what storing the version on the row is for.

**Boundary**: manifest checksums alone are **blind to version-history corruption**, which is why
both halves are verified. And the frozen column set **excludes** `lifecycle_state`,
`deprecation_provenance`, `replaced_by_sku_id` and `internal_revision` (**P-D-24**, extended by
**P-D-35**) — those move on transitions that write no version row, so the drill **must not expect
them** in the digested content.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and stepped in
[`../design/10-retention-erasure.md`](../design/10-retention-erasure.md) §3**.

### The identity-reference map

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-identity-map`

**Input**: a principal appearing at any door that stamps an `actor_ref`, and the erasure or age
trigger.

**Output**: a row of `products_identity_ref` at `(tenant_id, actor_ref)` carrying `principal_ref`,
the identity payload or its tombstone, `first_seen_at` and `last_seen_at`; and, at render time, the
resolution every projection and queue joins through.

**Boundary**: reads join through the map **at render time** and **no surface caches a resolved
identity**. The map is excluded from every export except the compliance surface — never `audit × export`
output.

**Three properties this process turns on, each of which was a correction.** `principal_ref` is
`NOT NULL` because three rules read the map **by principal** — erasure's resolve, the export's *"per
named principal"*, and the first-appearance predicate — and the key admitted no such read
(**P-D-49**). `last_seen_at` is advanced by every act that **resolves** the ref and not by minting
it, because minting happens once per active ref and the earlier wording pinned the column to
`first_seen_at` forever, tombstoning an active employee mid-employment. And a tombstoned ref is
**retired permanently**: re-minting that key for the same principal would make render-time joins
show the **new** identity against historical rows.

**And the property that makes it *"the only table in the gear where PII may live"* true at all** is that **every**
operator free-text field is inside the content-PII write block — not only attribute and description
text and the metadata map, but the `reason` fields on audit rows, approval rejections, break-glass
sessions, correction overrides, and the `SkuRetired` broker payload — **not** bulk and promotion
rows, which **P-D-50** struck from `design/02`'s canonical enumeration.
Personal data typed there would be **unreachable by erasure**, since those records are never edited
and erasure is a map-only tombstone. **Fail-closed at the door is the only reach erasure can have
over a record it may not rewrite.**

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-retention-errors`

**Input**: a refusal raised by a door of this feature.

**Output**: a `DomainError` variant carrying its wire code, and the RFC 9457 problem response
`design/10` §3.2 pins for it.

**The owned roster is one**, and it is normative at
[`../design/10-retention-erasure.md`](../design/10-retention-erasure.md) §3.2, which keeps
`cpt-cf-bss-products-contract-retention-errors`:

| code | status | raised by |
|---|---|---|
| `ERASURE_UNKNOWN_ACTOR` | **422 architectural → 400 on the wire** | the erasure door, when the named principal resolves to no `actor_ref` in this tenant |

**`CONTENT_PII_BLOCKED` is `02-taxonomy-attributes`' declaration**, kept out of the owned list by the
one-declaration rule and listed here **only for the response map** — it is also **422 architectural
→ 400**. `STALE_VERSION`, which appears in the slice's §3.2 note, is the **pricing** gear's and
reaches the **slice** only inside that note's quotation of D-141; this document names it in this
sentence and nowhere else.

**The 422s are architectural, not wire.** The slice states it: *"no `CanonicalError` category renders
422, so each reaches the wire as a 400 carrying its code, and no endpoint may declare a 422 for an
error **carrying a registry code** in `OpenAPI`"*. **The status is transcribed from §3.2's own
`Problem responses` block and MUST NOT be re-derived from a class rule.**

**The GC and the drill raise alarms, not API errors** — `retention_orphan_blocked` and the
corruption incident, neither of which is a code. **The digest-version mismatch is not an alarm**: §2
and §5 both make it a distinct result, and §7 row 7 routes its disposition, which no document
states.

## 4. States (CDSL)

**No state machine is declared by this feature.**

`design/10` declares no `state-` id, and the one lifecycle this feature does drive — a map row's
`live → tombstoned` — is **one-way, once, and irreversible**, expressed as a nullable column plus a
partial unique index rather than as a state column: `tombstoned_at` is *"Set once, by erasure, and
never cleared"*, and `uq_products_identity_ref_active` is what makes the transition observable. A
principal acting after its erasure **mints a fresh row** rather than moving the old one back.

Because §4 declares nothing, **this feature mints no `inst-` id at all**, as on
`features/catalog-version.md` and `features/reference-signal.md`.

## 5. Definitions of Done

Every DoD below names types, functions, tables and tests **that exist at `80eee534a`** wherever one
exists. Where the shipped seam already carries an obligation, the DoD says so rather than restating
it as new work.

### The identity-reference map's act, over a store that already ships

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-identity-map`

`products_identity_ref` **already exists** with the columns and guards this feature needs:
**and this DoD's remainder is scoped** (**P-D-72**): the tombstone-inclusive read is a **widening of
the ticked `cpt-cf-bss-products-dod-actor-ref`**, the function's owner — never a second DoD over the
same code — while this DoD keeps the erasure-resolve and export halves its siblings oblige.
`(tenant_id, actor_ref)` PK, `principal_ref NOT NULL`, nullable `identity_payload`, `tombstoned_at`,
`first_seen_at`, `last_seen_at`, `chk_products_identity_ref_tombstone`,
`chk_products_identity_ref_seen_order`, the `(tenant_id, principal_ref)` index and the partial unique
`uq_products_identity_ref_active`. **This DoD creates no table.**

**The read-by-principal path already ships, under another feature's ticked DoD.**
`repo::resolve_actor_ref` resolves a principal to its **at most one** live `actor_ref` — the partial
unique index caps it there — citing `inst-im-map` by name, and it is `features/foundation.md`'s
`cpt-cf-bss-products-dod-actor-ref`, which is **ticked**, with `@cpt-dod:…-dod-actor-ref:p1` on both
`products_identity_ref` source files. **P-D-49** records that the key admitted no read by principal
before `principal_ref` landed; it has since.

Two facts about that shipped function constrain this feature and neither is optional: it **mints on
a miss**, so the erasure door **MUST NOT** call it — an unknown principal would gain a fresh live
row instead of being refused — and it filters `tombstoned_at IS NULL`, so it cannot serve the
compliance export, which returns a principal's entries **including tombstoned ones**.

So what this DoD adds is a **second, tombstone-inclusive read** over the same
`(tenant_id, principal_ref)` index, for the export alone. What remains of the rest, given the ticked
DoD, is §7's.

`last_seen_at` **MUST** be advanced by every act that **resolves** a ref, as a **same-transaction
touch** rather than a separate act, and **MUST NOT** be advanced by minting alone. The entity's own
doc already states this; the DoD is that every stamping door honours it.

**Implements**: `cpt-cf-bss-products-algo-identity-map`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_identity_ref`
- Entities: `IdentityRefMap`

### The erasure door

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-erasure-door`

The system **MUST** serve `POST /bss-products/v1/erasure-requests` on `erasure × execute`, resolve
the named principal to its **at most one** live `actor_ref` **in the requesting tenant** — the
partial unique index caps it there — and **overwrite the map
entries with tombstones in one transaction** — destroying `identity_payload`, stamping
`tombstoned_at`, and leaving `principal_ref` standing. It **MUST** audit the act with its reason,
under the eraser's **own** pseudonymous ref.

It **MUST NOT** touch any immutable record (C1). `chk_products_identity_ref_tombstone` is what makes
a half-done tombstone impossible at the storage layer — it is **one-directional**, refusing a row
that carries both a payload and a `tombstoned_at` while admitting a payload destroyed with no
tombstone stamped, so the atomicity is the **one transaction** above and not the CHECK — and
`uq_products_identity_ref_active`'s partial predicate is what lets the same principal mint a **fresh**
row afterwards rather than reviving this one.

An unknown principal **MUST** be refused `ERASURE_UNKNOWN_ACTOR`, **naming the principal**.

**Erasure completes within one tenant** (**P-D-50**), and this DoD **MUST NOT** mint a platform-plane
grant to widen it.

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-03).

| clause | where it lands | what proves it |
|---|---|---|
| the route on `erasure × execute` | `api::rest::retention`'s router, `Gate::Erasure` | the door answers `200` and a denial is audited as `PERMISSION_DENIED` |
| resolve to **at most one** live ref **in this tenant** | `repo::tombstone_principal`, which carries its own resolve | `an_erasure_stops_at_the_tenant_boundary`: the same principal in a second tenant is untouched (P-D-50) |
| tombstone in **one transaction** | the door's `transaction_with_retry`, the audit inside it | `an_erasure_retires_the_ref_and_records_it` |
| destroy the payload, stamp the tombstone, leave `principal_ref` | `tombstone_principal`'s `UPDATE` | `an_erasure_destroys_a_seeded_payload_and_stamps_the_tombstone` — the payload is **seeded first**, because no production writer ever sets it and the case would otherwise be vacuous |
| audit with the reason under the **eraser's own** ref | `repo::write_evidential_act_audit` | `the_evidential_row_carries_the_erasers_ref_and_the_reason` |
| **MUST NOT** touch any immutable record (C1) | nothing outside the map and the audit log is written | `an_erasure_leaves_a_frozen_record_byte_identical` — §6's flagship, both halves in one probe |
| `ERASURE_UNKNOWN_ACTOR`, naming the principal | the miss returns `None`, which the door refuses | `an_unknown_principal_is_refused_and_nothing_is_minted`, whose second half is the one that matters: the shared actor context would have **minted** the unknown principal a ref |

**The door does not resolve the subject through the shared actor context**, and that is the DoD's own
constraint honoured rather than restated: `resolve_creator_actor_ref` mints on a miss, so an unknown
principal would gain a fresh live row and the door would report a successful erasure of a principal
it had just invented. The caller's ref comes from that context; the subject's does not.

**The reason rides the content-PII write block**, as `inst-av-pii-reason`'s enumeration requires of
every operator free-text `reason` — with the registered `NoPiiPolicyDetector` until
`dod-pii-detector` lands a real one, exactly as the product and SKU doors do.

**Implements**: `cpt-cf-bss-products-flow-erasure`

**Touches**:
- API: `POST /bss-products/v1/erasure-requests`
- DB Table: `products_identity_ref`, `products_audit_log`
- Entities: `IdentityRefMap`

### Age-based pseudonymization, as the same act

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-erasure-age`

The system **MUST** run the **same tombstone act** automatically at the configured maximum age,
emitting the **same** event. Erasure-on-request and erasure-on-age are **one mechanism with two
triggers**, and this DoD **MUST NOT** produce a second code path.

The age operand **MUST** be `last_seen_at` — the age of the principal's **last activity in the
tenant** — and **MUST NOT** be `first_seen_at`. Age-since-first-appearance would tombstone an active
employee mid-employment, which is the failure the column's semantics were corrected to prevent.

**Implements**: `cpt-cf-bss-products-flow-erasure`

**Touches**:
- DB Table: `products_identity_ref`, `products_audit_log`
- Entities: `IdentityRefMap`, `RetentionClock`

### The compliance export

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-compliance-export`

The system **MUST** serve `GET /bss-products/v1/compliance/identity-export` on **`compliance ×
export`** — **its own grant, never `audit × export`** — returning, per named principal, that
principal's map entries plus the audit-row references carrying their refs, DSAR-shaped. **Every
access MUST be individually audited.**

The separate grant is not stylistic: `design/10` §4 excludes the map from `audit × export`'s output, and this is
the one surface that returns **real identities**. Folding it into the audit grant would hand every
auditor the identities the whole pseudonymization scheme exists to withhold.

**Implements**: `cpt-cf-bss-products-flow-erasure`

**Touches**:
- API: `GET /bss-products/v1/compliance/identity-export`
- DB Table: `products_identity_ref`, `products_audit_log`
- Entities: `IdentityRefMap`

### The PII detector policy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-detector`

The system **MUST** answer `02-taxonomy-attributes`' hook with **block / allow / allow-by-list**, and
**uncertainty MUST block** (C2). A block **MUST** raise `CONTENT_PII_BLOCKED` **naming the field and
never the detected value** — a refusal that echoed the detected string would write the personal data
into the refusal's own audit row.

**The hook is 02's and this DoD MUST NOT relocate it**; only the policy is here.

**The door set that invokes it is wider than the hook's original scope**, and the DoD obliges the
whole set: every `reason`-bearing door — audit rows, approval rejections, break-glass sessions,
correction overrides, and the `SkuRetired` broker payload — raising the same code. **Not**
bulk and promotion row reasons: `design/02`'s canonical enumeration records that item **struck by
P-D-50**, since 09 has no free-text `reason` door of its own — its batch reason lives on 05's
approval and its mass-retire reason on 04's. `design/10` §3.1 still carries the struck item because
P-D-50's propagation never reached `inst-im-map`; that tail is its owner's, and §7 carries it. Personal data typed into any of them would be **unreachable by erasure**, because
those records are never edited and erasure is a map-only tombstone.

**`CONTENT_PII_BLOCKED` now has its `DomainError` arm, minted in `02` where the code is declared**,
and this DoD **MUST NOT** mint a second: minting one here would make this feature the second author of
another slice's code. §7 row 15 routed it and is **closed by measurement** — the arm landed on the
side that owed it, which is the outcome the row asked for and not a waiver of the rule.

**Implements**: `cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- Entities: `PiiDetector`

### The Legal-governed allow-list

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-allowlist`

The system **MUST** create `products_pii_allowlist` holding governed entries with their
justifications and a **mandatory Legal sign-off reference** — the artifact identifying the external
decision — per tenant, audited, and **exportable for the Legal review**. **An entry offered without
the reference is refused riding 01's `VALIDATION`**, the violation naming the missing field
(**P-D-64**): a missing mandatory member of the offered entry is a shape-class refusal, and no slice
code is minted for it.

A mutation **MUST** be a `GovernedLiveOp` on `pii_allowlist × write` under the **base approver
quorum** (**P-D-10** — there is no gear-side Legal role), and **MUST** emit `PiiAllowlistChanged`. An
entry offered **without** a sign-off reference **MUST** be refused.

**What this DoD deliberately does not claim**: the gear proves a Legal reference **was recorded**,
never that Legal approved. The control is the paper sign-off plus the export.

**Implements**: `cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- DB Table: `products_pii_allowlist`, `products_approval`
- Entities: `PiiDetector`

### The retention clocks

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-clock`

The system **MUST** compute expiry candidates per record class at the **statutory maximum** — never
"indefinite" (C3) — across frozen versions, catalog versions, audit rows, and the evidential stores
this feature owns the interplay for: approval records and decisions, break-glass sessions and
correction overrides, all audit-grade.

The audit class's rows carry `audit_id` as their address and, on a refusal, nullable `error_code` and
`attempted_key`, so a sweep **deletes whole rows and never leaves a refusal without its classifier**.

**And that sweep has no admitted `DELETE` today.**
`m20260829_000004_create_products_audit_log.rs` refuses every `DELETE` on both engines and names this
feature as the owner of the arm: *"The retention arm is owed to slice 10's `inst-rt-gc` and is to be
opened, in this file, as `OLD.written_at < <cutoff>` once `PRD` §15 supplies the window."* So the arm
lands **by editing that migration in place**, as `m20260829_000007`'s predicate does in
`06-catalog-version`, and it amends two shipped green tests —
`postgres_frozen_guards::the_audit_trail_admits_no_delete` and
`migrations_tests::a_delete_of_any_audit_row_is_refused_by_the_trigger`. The cutoff it needs is §7's:
a trigger cannot read configuration.

**Two populations are deliberately outside every clock**: outbox-delivered rows, whose horizon is the
**toolkit vacuum's** (**P-D-22**), and `07-reference-signal`'s watermark and member tables, which are
**operational current state**, continuously replaced.

**Implements**: `cpt-cf-bss-products-flow-retention`

**Touches**:
- DB Table: `products_audit_log`, `products_entity_version`, `products_catalog_version`,
  `products_approval`, `products_approval_decision`, `products_breakglass_session`,
  `products_correction_override`
- Entities: `RetentionClock`

### The retention gate, and the pair it evaluates

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-gate`

For a `catalogVersionId` the gate **MUST** range over **that version's `participant_set_snapshot`**
(**P-D-49**) and **MUST NOT** range over whatever registration rows exist. Over that domain it
**MUST** require **every** freeze registration to satisfy the **pair** — `state = released`, **or**
`state = not_frozen(forced)` **and** `released_at` stamped — and **MUST NOT** accept the timestamp
alone.

**Both halves of that sentence are corrections with a named failure behind them.** Quantifying over
registrations let an **empty ledger** satisfy the gate vacuously and collect a version nobody had
frozen. Reading the timestamp alone collected a version holding **live grandfathered references**,
because nothing clears the stamp: a forced participant that later recovered and acked left
`state = acked` beside a live `released_at`. A **snapshot member with no registration row holds** the
version — the fan-out has not reached it — while an **empty snapshot** is collectable, because nobody
ever owed an ack.

A candidate with a live registration **MUST** be **skipped** with `retention_orphan_blocked` — fail
closed, skipped and never forced (C4).

**This gate's operand is `06-catalog-version`'s** `inst-fz-liveness`, and that feature's §7 rows 6,
11 and 33 hold the open half — whether `freezeComplete`'s formula is restated to match this
predicate, the ledger's transition table, and who writes `released_at`. **Cited, not re-raised.**

**Built as `domain::retention`, and every case is armed as the failure rather than as its fix.** The
predicate takes the version's snapshot members and the ledger rows and answers `Collectable` or
`Held(..)`; it deletes nothing, and every hold carries C4's single reason so a caller cannot read
one as a soft warning. Six of the eight probes are the `DoD`'s own named failures:

- an **empty ledger** against a non-empty snapshot **holds** — the vacuity that collected a version
  nobody had frozen;
- an **empty snapshot** is **collectable** — the other vacuity, admitted, and the two differ in
  which store is empty;
- a **stamp beside a live state** (`acked` with `released_at` set — the recovered forced
  participant) **holds**, which is the failure the pair exists for;
- a **door-released** row satisfies the gate **with the stamp NULL**, so a gate reading the
  timestamp would have refused every ordinary release — the same defect from the other side;
- the **forced arm without its stamp** holds, and that arm reports a row that reached the table past
  its own shape `CHECK`;
- **every** hold is reported rather than the first, because an operator repairing one and re-running
  would otherwise meet the rest one pass at a time.

`FreezeRegistration` carries the state and the stamp **separately** rather than deriving one from
the other, and that is the shape the two arms force: a door-released row is `released` with a NULL
stamp while a forced row carries both (**P-D-67** — *"the state moving is what makes the stamp
inert"*).

**Implements**: `cpt-cf-bss-products-flow-retention`

**Touches**:
- DB Table: `products_freeze_ack`, `products_catalog_version`
- Entities: `RetentionGate`

### Deletion order, and the one edge that is physically enforced

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-order`

Deletion **MUST** respect reference topology: capture and entry rows before their catalog-version
row, entity versions only after every referencing manifest. Every GC act **MUST** be audited with the
class, the clock and the gate verdict.

**The last edge is physically enforced rather than merely ordered.** `01-foundation` admits an
entity-version `DELETE` only when no manifest entry references the row (**P-D-40**), and
`06-catalog-version`'s `cpt-cf-bss-products-dod-referential-delete-predicate` is what **replaced**
the unconditional refusal with that predicate, **by editing
`m20260829_000007_create_products_entity_version.rs` in place**, and it is ticked. **06 specified the
guard; this DoD owns the deleter that spends it** — and the `DELETE` this GC needs is admitted today,
which is why §7 row 17 is closed. What remains open on this DoD is rows 5 and 25, neither of which is
about the guard.

An entity-version row referenced by **any** retained catalog-version manifest **MUST** be retained
with it: version-row retention **derives** from catalog-version retention and is **never shorter**.

**Implements**: `cpt-cf-bss-products-flow-retention`

**Touches**:
- DB Table: `products_entity_version`, `products_catalog_version_entry`, `products_catalog_version`
- Entities: `RetentionClock`, `RetentionGate`

### The restore drill

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-restore-drill`

On the configured cadence the system **MUST** restore a sampled set of catalog versions **and their
referenced entity versions** from backup into an **isolated target**, and re-verify **both** the
manifest checksums and the per-row `content_digest` **byte-for-byte** (C5). Manifest checksums alone
are **blind to version-history corruption**, which is why both halves are named.

The drill **MUST** compare **like with like**: each frozen row carries the `digest_version` it was
computed under, and re-verifying a row written under an earlier version is a **version mismatch**,
not a corruption alarm. `canonical::DIGEST_VERSION`'s own doc gives this drill as the reason that
column exists.

It **MUST NOT** expect `lifecycle_state`, `deprecation_provenance`, `replaced_by_sku_id` or
`internal_revision` in the digested content (**P-D-24**, extended by **P-D-35**) — those move on
transitions that write no version row.

A byte mismatch **MUST** raise a **compliance incident alarm**, not a log line. Results **MUST** land
on an operator surface carrying the **last-verified watermark per tenant** — see §7 for where that
watermark lives, which no document states.

**The seam this DoD stands on is already agreed in code.** `m20260829_000007` chose `text` over
`jsonb` precisely so a row cannot be *"perfectly intact and still fail slice 10's restore drill"*,
and `repo.rs` names one of its guarantees *"the single property slice 10's restore drill depends
on"*.

**Implements**: `cpt-cf-bss-products-flow-restore-drill`

**Touches**:
- DB Table: `products_entity_version`, `products_catalog_version`
- Entities: `RetentionGate`

### The error taxonomy

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-error-taxonomy`

The system **MUST** add a `DomainError` variant for `ERASURE_UNKNOWN_ACTOR` carrying its wire code
through `DomainError::code`, and **MUST** give it the RFC 9457 response `design/10` §3.2 pins —
**422 architectural, reaching the wire as a 400**.

**One code is the whole owned roster**, and **P-D-64** keeps it so: the allow-list's
missing-sign-off refusal rides 01's `VALIDATION` with the violation naming the field, rather than
minting a second code nothing discriminates on. That is a measurement rather than an omission:
`CONTENT_PII_BLOCKED` is 02's declaration and the GC and drill raise **alarms, not API errors**.

`DomainError::code` is exhaustive, so the variant itself is compile-gated. **Five hand-written sites
are not**, and they are named as lines because a blanket criterion is ticked by inspection:
`infra::error_mapping`'s `From<DomainError> for CanonicalError` ladder gains an arm;
`error_mapping_tests::DOMAIN_ERROR_VARIANTS` is bumped; `one_of_every_variant` and
`declared_status_and_code` each gain a row; and — because this code is **422 architectural** —
`error_mapping_tests::the_products_owned_422_codes_stay_wire_400_by_design`'s **hard-coded
three-element array** gains a fourth. `features/reference-signal.md` names the first four and
`features/catalog-version.md` names the fifth; without the fifth this code lands unguarded, and the
day someone attaches a 422 transport override to it every test stays green.

**Implements**: `cpt-cf-bss-products-algo-retention-errors`

**Touches**:
- Entities: `IdentityRefMap`, `PiiDetector`

### The authz surface, and the four rosters it reddens

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-authz`

The system **MUST** declare the labels and actions this feature's three grants spend — `erasure ×
execute`, `compliance × export` and `pii_allowlist × write` — **MUST** declare one
`crate::authz::resource_types` descriptor per new label, and **MUST** register a permission instance
per `(resource_type, action)` pair.

**Four shipped roster tests bound this, one of them positionally**, and each **MUST** be updated in
the same change — named as lines because a blanket criterion is ticked by inspection:

- `authz_tests.rs` asserts `labels::ALL == [labels::PRODUCT, labels::SKU]`, a **positional**
  equality, so a third label reddens it.
- `gts::permissions`' `EXPECTED_PERMISSION_IDS` holds exactly six ids, compared as a set **in both
  directions**, with a separate length check.
- `catalog_resource_types_match_authz_labels_all` asserts the catalog's distinct `resource_type`s
  are **exactly** `labels::ALL`.
- `catalog_actions_are_declared_action_constants` compares each action against a **hard-coded**
  `known = [READ, WRITE, PUBLISH]`, so `execute` and `export` must be added to **that array** as
  well as to `actions`.

**The descriptor clause is not optional and nothing reddens without it**: `authz.rs` requires that
*"every authoring door passes one of these, never a bare label string, to `access_scope`"*, and its
descriptor test asserts only the two labels that exist.

**`pii_allowlist × write` carries "no route declared"** in `design/05-governance.md` §3.2, and
`features/governance.md` §7 row 1 holds that gap open across eleven grants. **Cited, not decided
here** — so this DoD declares the grant and does not invent the door.

**Built, and the four roster tests were four different shapes of census.** Each of the four
sentences above quoted the roster as it stood when this DoD was authored, and every one of those
numbers had moved before this change landed — `labels::ALL` held **eleven** entries, not two;
`EXPECTED_PERMISSION_IDS` held **twenty-one** ids, not six; `known` held **eight** actions, not
three. The obligation the sentences carry survived the drift intact, because it names the *sites*
rather than the counts: the positional equality, the two-way set comparison, the
resource-types-match assertion and the hard-coded action array. All four are updated here, and none
of the three grants needed a new action — `execute`, `export` and `write` already exist, so the
**resource is the discriminator** and the action vocabulary does not grow.

Three lines also left the catalog census's **absence** list, where an earlier group had asserted
them missing with `10` named as the owing slice. That was the right assertion then and is the wrong
one now: this feature's own `DoD` declares them, which is exactly the rule the census encodes.

**Implements**: `cpt-cf-bss-products-flow-erasure`, `cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- Entities: `IdentityRefMap`, `PiiDetector`

### The two events, and the one that is deliberately minimal

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-events`

The system **MUST** emit `ActorErased(actor_ref)` and `PiiAllowlistChanged`, each in the same
transaction as its act and each carrying a **versioned** schema reference. Both `THE_EIGHT` rosters —
`infra::events::events_tests` and `infra::broker::broker_tests`, neither derived from the code —
**MUST** be extended in the same change.

**`ActorErased` carries the ref and no identity**, and its consumer set is **legitimately empty
today**: no projection in the design set materializes identities — renders join the map, and
`08-read-models` holds pseudonyms only — so the event is a **defensive cache-buster** rather than a
contract anyone consumes. **Materializing an identity into any projection is a `12-consumer-contracts`
lint failure** — its **Lint 7**, which that feature declares by name and which names this feature's
`IdentityRefMap` as the one permitted store.

**Neither event fits `EventBodyCore`.** An `actor_ref` and an allow-list entry have none of its five
fields, `EntityKind` is exactly `Product | Sku`, and the only subject types declared are the Product
and SKU ones. They need the same entity-less core `features/catalog-version.md` §7 row 27 registers
and the `SUBJECT_TYPE` its **row 47** raises — **cited, not re-raised**. Their `aggregate_id`, which
`infra::events::enqueue` requires and `partition_for` consumes, is reached by neither row and is
§7's here.

The erasure act itself, the GC's deletes and the drill's results are **audit-plane with an explicit
no-broker-event carrying identity**: an event carrying identity is exactly what this feature exists
to prevent, and `ActorErased` deliberately carries none.

**Implements**: `cpt-cf-bss-products-flow-erasure`,
`cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- Entities: `IdentityRefMap`, `PiiDetector`

## 6. Acceptance Criteria

**The reproducibility-vs-erasure flagship**

- [ ] Freeze a version, erase its approver, then assert **both halves in one probe**: the old
      snapshot's checksum is **unchanged**, and the rendered audit shows the **tombstone**. C1 is
      only proven by asserting both — either half alone passes on a build that got the other wrong.

**The detector**

- [ ] A matrix of block / allow / allow-by-list / uncertainty-blocks, **each with a positive
      control**, so no arm passes because the fixture could not reach the permissive branch.
- [ ] A block names the **field** and the assertion checks the refusal does **not** carry the
      detected value.
- [ ] Every `reason`-bearing door in the enumerated set raises the same code: audit rows, approval
      rejections, break-glass sessions, correction overrides, and the `SkuRetired` payload — and
      **not** bulk or promotion rows, which P-D-50 struck.

**The allow-list**

- [ ] A mutation runs the **base quorum** and is refused when the Legal sign-off reference is
      absent, **asserted with its positive control** — a mandatory-field rule proven only by its
      refusal is a rule that may never admit anything.
- [ ] An admitted entry is exportable for the Legal review.

**The retention gate, RED first**

- [ ] A candidate version with one live freeze-registration is **skipped and alarmed**.
- [ ] The same version GCs cleanly once **every** registration satisfies the pair — `state =
      released`, or `state = not_frozen(forced)` with `released_at` stamped.
- [ ] A registration at `state = acked` **beside a stamped `released_at`** is **still skipped and
      alarmed**. This is the correction's own regression probe: reading the timestamp alone collects
      a version holding live grandfathered references.
- [ ] An **empty** `participant_set_snapshot` is **collectable**; an empty **registration ledger**
      under a non-empty snapshot is **not**. The two are asserted apart, because quantifying over the
      wrong one satisfies the gate vacuously.

**Derived retention and order**

- [ ] An entity-version row referenced only by a **retained** catalog version survives its own class
      clock.
- [ ] A GC attempting an entity-version `DELETE` while a manifest entry still references it is
      refused **by the guard**, not merely skipped by the GC — the probe passes even when the GC is
      bypassed entirely.

**The drill**

- [ ] A deliberately **corrupted** backup sample fails the drill loudly. The oracle must be seen to
      fail.
- [ ] A row written under an earlier `digest_version` produces a **version mismatch**, distinguished
      from a corruption alarm in the result, not only in a log line.
- [ ] The drill does **not** expect `lifecycle_state`, `deprecation_provenance`,
      `replaced_by_sku_id` or `internal_revision` in the digested content.

**Erasure**

- [ ] Age-based pseudonymization fires **without a request** and produces the same map state as the
      requested path. What "byte-identical in effect" covers beyond map state is §7's.
- [ ] A principal acting **after** its erasure mints a **fresh** ref rather than reviving the
      tombstoned row, and render-time joins of historical records still show the tombstone.
- [ ] A repeat DSAR **after** an erasure still resolves by principal — which is what
      `principal_ref` surviving the tombstone is for.
- [ ] `last_seen_at` advances on a **resolve** and not on a mint, asserted directly, since the
      age trigger reads it.

**Positive control, one line per declared code** — one code, one line.

- [ ] `ERASURE_UNKNOWN_ACTOR` — a principal with no `actor_ref` in this tenant is refused **naming
      the principal**; the same request for a principal that has one succeeds.

**Controls on the shipped seam**

- [ ] `chk_products_identity_ref_tombstone` refuses a row carrying both a payload and a
      `tombstoned_at`, on **both** engines.
- [ ] `uq_products_identity_ref_active` admits a second row for the same
      `(tenant_id, principal_ref)` **once the first is tombstoned**, and refuses it while the first
      is live. Both arms, because the partial predicate is the whole mechanism.

## 7. Known unknowns

**The arithmetic of this section.** Thirty-two rows: **fourteen carried verbatim** from
[`../design/10-retention-erasure.md`](../design/10-retention-erasure.md) §6 — the slice's full count,
not a selection — and **eighteen raised here**: five while authoring, from reading the crate, and
thirteen by the three-lens review of this document. Of the thirty-two, **fourteen block no DoD in this
document** (rows 1, 2, 3, 30, 31 and 32; row 13, resolved by **P-D-64 on 2026-08-31**, and row 20,
resolved by **P-D-72 on 2026-09-01**; rows 15, 17, 19 and 29, **closed by measurement on
2026-09-03**; and rows 4 and 24, **answered by the owner on 2026-09-03** — all eight of those kept in
place rather than struck); the other eighteen each name the DoD they block. **A row marked *closed by measurement* was answered by nobody**: no decision was taken and
none is owed, the crate simply moved past the row's premise, and the entry says which fact it moved
past so the close is checkable. That is a different disposition from a resolved row and is spelled
differently on purpose. Row 8 is **parked for the owner** with `features/read-models.md`'s
row 25 — the same identity-map privacy fork — so `dod-identity-map` waits on it. A third
subsection below carries one-line pointers into other documents' registers; those are not rows.

**Carried, not answered**, and registered against **its owner's** register. **Three departures from
verbatim, declared so the claim is checkable.** First, the slice's inline `Owner:` sentence and its
provenance marker are converted into this document's `**Owner**:` field. Second, **rows 1, 2 and 3
carry no `Owner:` sentence in `design/10` §6 at all** — their `**Owner**` field is authored here, and
their routing has not been agreed with the parties named. Third, every bare `§N` inside a carried row
is **`design/10`'s numbering, not this document's**: §1.1, §1.5, §3.1, §3.2, §4 and §5 there are the
slice's sections, and three of the six do not exist here. Apart from those three classes the carried
text was diffed against `design/10` §6 sentence by sentence, mechanically, and **eleven of the
fourteen rows are byte-identical**. The other three have moved since the carry and each says so in
place: **row 7** gains a re-measurement at `80eee534a`, **row 11** replaces an entanglement with an
item **P-D-50** struck, and **row 13** rewords its P-D-64 answer. Re-diffed 2026-09-03; an earlier
version of this paragraph claimed nothing beyond the three classes was altered, and that was false in
those three rows.

**Three questions are deliberately NOT raised here because a sibling already owns them**, and are
cited instead:

- the `freezeComplete` formula, the freeze-ledger transition table, and who writes `released_at` —
  `features/catalog-version.md` §7 rows 6, 11 and 33, cited by
  `cpt-cf-bss-products-dod-retention-gate`, whose predicate is the other half of all three;
- the entity-version `DELETE` predicate this feature's GC spends —
  `features/catalog-version.md`'s `cpt-cf-bss-products-dod-referential-delete-predicate`, which
  obliges it — **row 9 is closed** (**P-D-60**): the capture store is its own table, so the predicate
  judges a population whose every row references an entity version and needs no re-aiming;
- whether §6 owes one criterion per DoD — `features/catalog-version.md` §7 row 50.

### Carried verbatim from `design/10` §6

1. **Detector quality is a product risk, not a design one**: fail-closed-on-uncertainty guarantees
   safety and guarantees friction; the allow-list loop must exist before GA (the 02 risk restated as
   this slice's operational owner), and the §15 Legal sign-off covers the posture itself.
   **Blocks**: no DoD — it is a product and operational risk.
   **Owner**: this feature, operationally.

2. **Watermark/member tables (07) and bulk ledgers** carry `skuId`s and row payloads, not PII —
   asserted here so their retention rides ordinary classes; if a future producer's payload grew
   identity-bearing fields, the map discipline would apply — named to keep it from drifting in
   silently.
   **Blocks**: no DoD — it is an assertion recorded to stop a silent drift.
   **Owner**: this feature, with `07-reference-signal` and `09-bulk-promotion` if a payload widens.

3. **Encrypted-at-rest for the map** rides the platform storage posture; if a deployment lacks it,
   this table is the one that must not ship — a deployment gate, not a code path.
   **Blocks**: no DoD — it is a deployment gate.
   **Owner**: the platform storage owner.

4. ~~**Which store holds the audit rows this slice's own rules require?**~~
   **Answered (owner call, 2026-09-03) in two parts, because the row holds two different acts.**
   **The compliance export needs no new class**: P-D-21 words class 2 as *"reads under elevation"*
   and justifies it as *"a read writes no outbox row at all"*, and the export is a read that writes
   no outbox row, so the class widens from its example to its own stated reason. **The erasure act
   gets a fourth class** — *"acts whose evidential record must carry a field the event deliberately
   omits"* — the row's other two arms both being closed by this document's own `dod-retention-events`,
   which forbids widening `ActorErased` and requires it be emitted. Built as
   `repo::write_audited_read_audit` and `repo::write_evidential_act_audit`; until
   `dod-retention-events` lands the erasure act emits nothing and class 3 covers it verbatim, the
   fourth class being what keeps it admitted afterwards. **`01` §4.4 owes the fourth class's
   sentence**, which is `01-foundation`'s edit and not this feature's.
   Original text: `inst-er-erase` is "audited
   with a reason", `inst-er-export` audits "every access individually", and 01 §4.4 holds — under
   **P-D-21** — "only acts that emit no event, in three classes". The erasure act emits
   `ActorErased`, so it writes no row there; the compliance export is a read that is not "a read
   under elevation"; and the minimal `ActorErased(actor_ref)` carries neither the reason nor the
   eraser's own ref. Either 01 §4.4 gains a class, `ActorErased` widens, or these acts are declared
   eventless.
   **Blocks**: no DoD — **answered**; `01` §4.4's sentence is owed to `01-foundation`.
   **Owner**: was `01-foundation`'s owner with P-D-21's; **answered, one edit owed**.

5. **What `actor_ref` attributes an unattended act's audit row?** The age-triggered tombstone and
   every GC act are audited, 01 makes the audit row's `actor_ref` non-nullable, and every ref in the
   set is minted for a human principal on first appearance. No document names a system ref or admits
   a null one.
   **Blocks**: `cpt-cf-bss-products-dod-erasure-age`,
   `cpt-cf-bss-products-dod-retention-order`.
   **Owner**: `01-foundation`'s owner with this feature.

6. **Where does a successful restore drill's state live?** `inst-rd-drill` puts "the last-verified
   watermark per tenant" on an operator surface; §4 says "retention/drill state is config + audit, no
   new record tables"; and a per-tenant watermark is neither config nor an append-only audit row.
   `DECISIONS.md` already records this as owed under P-D-21 and deliberately unapplied.
   **Blocks**: `cpt-cf-bss-products-dod-restore-drill`.
   **Owner**: this feature with P-D-21's owner.

7. **What does the drill do on a digest-version mismatch, and what is the corruption alarm called?**
   The mismatch arm is named and not terminated — nothing says whether the row is skipped,
   re-rendered under its stored version, or reported — the alarm is unnamed where every other alarm
   in the set has a name, and the sample rule ("a sampled set") is unstated. *(Re-measured at
   `80eee534a`: `canonical::DIGEST_VERSION`'s doc constrains the answer — a bump "must arrive with
   the code that can still recompute the old rendering for those rows", or the drill "re-verifies
   every historical row against a rule it was never computed under and reports the whole table
   corrupt". What stays open is what the drill does when that code is absent.)* 01 pins `digest_version`
   starting at `1` as a code constant, so no second version yet exists to test the arm against.
   **Blocks**: `cpt-cf-bss-products-dod-restore-drill`.
   **Owner**: this feature with the NFR #5 workshop.

8. **Which surfaces may resolve an identity through the map, and under what grant?**
   `compliance × export` is "its own grant, never `audit × export`", and it is the only grant any
   document attaches to a map read — yet `inst-im-render` has approval queues and 08 projections
   resolving at render time. 08 states it renders "actor pseudonyms" and never mentions the map or a
   join, so the two slices disagree about what 08 does. 12 forbids *storing* an identity elsewhere
   and says nothing about resolving one.
   **Blocks**: `cpt-cf-bss-products-dod-identity-map`,
   `cpt-cf-bss-products-dod-compliance-export`.
   **Owner**: `05-governance`'s RBAC catalog owner with this feature and `08-read-models`.

9. **Who owns NFR #5's cold re-resolution MUST, and how does the clause split with 06?** Both slices
   claim the requirement as "shared", while 12 requires exactly one owner per clause. The word "cold"
   appears nowhere in the design set except this slice's Traces-to line: there is no instruction, no
   §4 shape and no §5 probe for cold resolution or its p95, and 06 — which owns the resolver — claims
   only the durability half.
   **Blocks**: `cpt-cf-bss-products-dod-restore-drill`.
   **Owner**: the design-set owner with `06-catalog-version`.

10. **Who delivers the DR half of the durability mechanics?** §1.1 promises "storage class, periodic
    checksum restore verification, DR posture" and §1.5 "checksum restore verification cadence, DR
    posture as config + probes", and the only body rule is the drill; storage class and RPO/RTO
    appear only as a constraint deferred to the NFR workshop. Either an instruction owes the DR
    config and probes, or Scope In owes a narrowing.
    **Blocks**: `cpt-cf-bss-products-dod-restore-drill`.
    **Owner**: the design owner, with the NFR workshop's output in hand.

11. **Which actor holds `compliance × export`?** The door returns real identities, the only
    compliance actor in §1.3 is described as reading "over pseudonymized trails", and 05's RBAC
    catalog has no such pair for this gear. Entangled with the erasure reach this feature's §2 closes at per-tenant
    (**P-D-50**).
    **Blocks**: `cpt-cf-bss-products-dod-compliance-export`.
    **Owner**: Architecture with Legal.

12. **Is `products_pii_allowlist` itself a PII store?** By construction it holds person-named
    strings, and its entries are "exportable for the Legal review", yet only the map carries the
    posture "excluded from every export except the compliance surface, encrypted at rest". The same
    question decides whether the allow-list's justification and sign-off fields belong in §3.1's
    content-PII write block — this pass synced that enumeration to 02's canonical list and
    deliberately did **not** add these two fields.
    **Blocks**: `cpt-cf-bss-products-dod-pii-allowlist`,
    `cpt-cf-bss-products-dod-pii-detector`.
    **Owner**: Legal with the data-protection owner.

13. ~~**What code does the allow-list's missing-sign-off refusal carry?**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-64): it rides 01's `VALIDATION`.** A
    missing mandatory member of the offered entry is a shape-class refusal, the caller's
    discriminator is the violation's **field**, and the SDK enum's `VALIDATION` member is the member
    this refusal uses — the owned roster stays at one code, which this document's own taxonomy DoD
    holds as a measurement. P-D-52's counter-precedent does not transfer: nothing discriminates on
    this code. Original text: `inst-pp-allowlist` refuses
    an entry offered without a Legal sign-off reference, §5 asserts that refusal with a positive
    control, and §3.2 declares no code for it — so the door answers unclassified and the SDK error
    enum has no member for a refusal a caller will routinely hit. Either it rides 01's `VALIDATION`
    or this slice declares its own.
    **Blocks**: no DoD — **resolved by P-D-64**;
    `cpt-cf-bss-products-dod-retention-error-taxonomy` is freed, and
    `cpt-cf-bss-products-dod-pii-allowlist` carries the answer while staying blocked by rows 12
    and 23.
    **Owner**: was this feature with the error-contract owner; **closed**.

14. **What does "byte-identical in effect" mean for the age-triggered path?** §5 asserts the age path
    is byte-identical in effect to the requested path, while the requested path is "audited with a
    reason" and the age path has no requester and no supplied reason. Nothing says whether the audit
    row is part of "effect" or only the map state.
    **Blocks**: `cpt-cf-bss-products-dod-erasure-age`.
    **Owner**: this feature.

### Raised here rather than carried

Five, from reading the crate at `80eee534a`. Every quotation was byte-verified against source.

15. ~~**`CONTENT_PII_BLOCKED` is raised by every door this feature's detector guards and is armed
    nowhere.**~~
    **Closed by measurement (2026-09-03, no decision taken): the arm landed in `02`, which is where
    this row routed it.** `domain::error::DomainError::ContentPiiBlocked` ships, its `code()` arm
    renders `CONTENT_PII_BLOCKED`, and `infra::error_mapping`'s ladder carries the matching arm at
    422 architectural. The row's **must not** was honoured rather than overtaken: the code was minted
    by the slice that declares it and not here. Original text: `domain::error::DomainError`'s
    fourteen variants carry no arm for it, and
    `infra::error_mapping`'s ladder therefore has none either — while `ILLEGAL_FIELD_MUTATION` and
    `STALE_REVISION`, other cross-slice codes, are armed. This feature **must not** mint it: the code
    is `02-taxonomy-attributes`' declaration and minting it here would make this the second author of
    another slice's code. The same gap is registered by `features/reference-signal.md` §7 row 18 from
    its own three reasons; **this row records the wider door set** the detector policy obliges.
    **Blocks**: no DoD — **closed by measurement**; `cpt-cf-bss-products-dod-pii-detector` stays
    blocked by rows 12, 22 and 23.
    **Owner**: was `02-taxonomy-attributes`, which declares the code; **discharged**.

16. **`repo::AuditCommon::correlation_id` is permanently `NULL`, and this feature owns the audit
    class's clock and its deletes.** The field is `Option<Uuid>` and its own doc names the blocker:
    *"the value is 32 hex characters and this column is `uuid` on Postgres"*, with two candidate
    migration shapes written up and the decision, the migration behind it and the wiring recorded as
    **owed**. A retention sweep over the audit class deletes rows whose correlation column never
    carried a value, so the class this feature retains is less useful than its schema implies. The
    debt is `01-foundation`'s.
    **Blocks**: `cpt-cf-bss-products-dod-retention-clock` — not its correctness, its worth.
    **Owner**: `01-foundation`'s code. One-line pointer only.

17. ~~**The GC has no admitted `DELETE` until `06-catalog-version`'s predicate lands.**~~
    **Closed by measurement (2026-09-03, no decision taken): the predicate landed.**
    `cpt-cf-bss-products-dod-referential-delete-predicate` is ticked in
    `features/catalog-version.md`, and `m20260829_000007_create_products_entity_version.rs` now
    refuses a `DELETE` only where a `products_catalog_version_entry` row references it — the Postgres
    trigger inside an `IF EXISTS` guard, the SQLite trigger under a `WHEN EXISTS` clause, and the
    file's own header rewritten to say the owed arm is paid. So
    `cpt-cf-bss-products-dod-retention-order` is no longer sequenced behind a DoD in another FEATURE.
    Original text: `m20260829_000007_create_products_entity_version.rs` refuses **every** `DELETE`
    unconditionally today and says why — the referential predicate's table did not exist — and
    `06-catalog-version`'s `cpt-cf-bss-products-dod-referential-delete-predicate` is what replaces it,
    **by editing that migration in place**. So this feature's `cpt-cf-bss-products-dod-retention-order`
    is **sequenced behind** a DoD in another FEATURE rather than merely dependent on it, and the two
    shipped green tests that pin the unconditional refusal are amended there, not here.
    **Blocks**: no DoD — **closed by measurement**; `cpt-cf-bss-products-dod-retention-order` stays
    blocked by rows 5 and 25.
    **Owner**: was `06-catalog-version` — sequencing, not a question; **discharged**.

18. **The map's read-by-principal path has no index of its own for the tombstoned case.**
    `idx_products_identity_ref_principal` covers `(tenant_id, principal_ref)` and
    `uq_products_identity_ref_active` covers the live subset, so resolving a principal's **historical**
    refs — which the compliance export needs, since it returns "the principal's map entries" and a
    tombstoned entry is still an entry — walks the non-partial index. Whether that is adequate is a
    sizing question no document asks, and the export is the one surface with a latency budget nobody
    has stated.
    **Blocks**: `cpt-cf-bss-products-dod-compliance-export`.
    **Owner**: this feature with the schema owner.

19. ~~**`ActorErased`'s consumer set is called "legitimately empty", and nothing enforces that it
    stays so.**~~
    **Closed by measurement (2026-09-03, no decision taken): both halves of the premise moved.**
    `features/consumer-contracts.md` declares **Lint 7 — identity materialization**, which names this
    feature's `IdentityRefMap` as the one permitted store and cites erasure's guarantee as its reason,
    with **P-D-45** supplying the lint's reading rule; and `08-read-models` **is written**, rendering
    actor pseudonyms and no identity. What is left — that no job runs the lints — is `12`'s own open
    item, registered there. Original text: The event is a defensive cache-buster because no projection
    materializes identities, and
    `12-consumer-contracts` is named as the lint that would catch one that did. But this feature
    declares no obligation on that lint and 08 is unwritten, so the property the event's minimality
    rests on is asserted by this document and enforced by nothing yet.
    **Blocks**: no DoD — **closed by measurement**; `cpt-cf-bss-products-dod-retention-events` stays
    blocked by rows 4 and 26.
    **Owner**: was `12-consumer-contracts`, with `08-read-models`; **discharged**.


20. ~~**What does `cpt-cf-bss-products-dod-identity-map` still oblige, now that the read path is a
    ticked foundation DoD?**~~
    **Answered (owner call, 2026-09-01 — P-D-72 arm 4): the tombstone-inclusive read is a widening of
    the ticked foundation DoD**, `cpt-cf-bss-products-dod-actor-ref`, the function's owner — a second
    DoD over the same code has two owners and no recorded precedence, this row's own argument. This
    DoD keeps the erasure-resolve and export halves, which their own DoDs already oblige.
    Original text: `repo::resolve_actor_ref` ships under
    `features/foundation.md`'s `cpt-cf-bss-products-dod-actor-ref`, which is `[x]`, and both
    `products_identity_ref` source files carry its `@cpt-dod` marker. Of the three rules this DoD
    named, the first-appearance predicate is that function, and the erasure resolve and the export
    are obliged by their own DoDs here. What is left is the tombstone-inclusive read — whether that
    is a DoD of its own, a widening of the ticked one, or nothing, has no stated answer, and a
    second DoD over the same code has no recorded precedence.
    **Blocks**: no DoD — **resolved by P-D-72**; `cpt-cf-bss-products-dod-identity-map` carries the split while staying blocked by row 8, parked.
    **Owner**: was this feature with `01-foundation`; **closed**.

21. **Does a principal have at most one live `actor_ref` per tenant, and is the erasure act therefore
    single-row?** The partial unique index caps live rows at one and the shipped resolve uses
    `.one(...)`, but `design/10`'s `inst-er-erase` — the normative step — says *"resolves the
    operator identity to its `actor_ref`s and overwrites the map entries"*, plural. Correcting the
    plural here would put this document against a step it declares normative.
    **Blocks**: `cpt-cf-bss-products-dod-erasure-door`.
    **Owner**: `design/10`'s owner, on `inst-er-erase`'s wording, with the schema owner.

22. **When a door-set fact differs between `design/10` §3.1 and `design/02`'s canonical enumeration,
    which governs?** §1.4 pins precedence for column-level facts only. The bulk/promotion reason
    entry is a door-set fact: `design/02` records it **struck by P-D-50**, and `design/10` §3.1 still
    carries it because P-D-50's propagation reached `inst-er-export`, `inst-er-erase` and §6 — never
    `inst-im-map`. This document follows `design/02`; nothing states that it should.
    **Blocks**: `cpt-cf-bss-products-dod-pii-detector`.
    **Owner**: `design/10`'s owner with `design/02`'s.

23. **What does an allow-list entry match on?** No document names the table's central column, nor how
    "allow-by-list" matches a candidate free-text value, nor what makes the detector *uncertain* such
    that C2's fail-closed arm fires. Without the matched value and the match rule, the third arm of
    the detector cannot be built and §6's four-arm matrix has no fixture.
    **Blocks**: `cpt-cf-bss-products-dod-pii-allowlist`,
    `cpt-cf-bss-products-dod-pii-detector`.
    **Owner**: Legal with the data-protection owner for what an entry may contain — row 12's pair —
    and `02-taxonomy-attributes` for how it is evaluated.

24. ~~**Does an erasure request or a DSAR export name a `principal_ref` or a real-world identity?**~~
    **Answered (owner call, 2026-09-03): the request names a `principal_ref`, and the
    person-to-pseudonym step belongs to whichever identity provider minted the principal.** This
    door is internal to the gear. The alternative was disqualified by the row's own argument: an
    identity string makes the refusal *"naming the principal"* write personal data into its own
    audit row, the failure `dod-pii-detector` forbids for `CONTENT_PII_BLOCKED`.
    **The row's objection to the chosen arm was measured false before the call**: `principal_ref` is
    `NOT NULL` and survives the tombstone by design (**P-D-49**), so a repeat DSAR still resolves;
    and a **first** DSAR from a principal that never held a ref resolves to *"no entries"*, which is
    a correct answer rather than a failure. `inst-er-erase`'s *"resolves the operator identity"*
    owes a wording fix, which is `design/10`'s and is row 21's neighbour.
    Original text: The
    store's key is the pseudonym. If the request carries `principal_ref`, nothing maps a person to
    one and a first DSAR for an already-erased principal is unresolvable, since the payload is gone.
    If it carries an identity string, the resolve searches an unindexed nullable column and the
    refusal *"naming the principal"* writes personal data into its own audit row — the failure
    `cpt-cf-bss-products-dod-pii-detector` forbids for `CONTENT_PII_BLOCKED`.
    **Blocks**: no DoD — **answered**; `design/10`'s `inst-er-erase` owes a wording fix.
    **Owner**: was Architecture with Legal — row 11's pair; **answered**.

25. **What is the GC's transaction boundary, and what does a crash mid-order leave?** The stated
    order makes an intermediate state observable — a catalog-version row surviving with its entry
    rows deleted — in which the referential guard now *admits* deleting entity-version rows the
    surviving manifest still names, and a backup taken in that window carries a partial manifest the
    drill reports as a compliance incident. Nothing scopes the GC's transaction, says whether a pass
    is per-version or per-class, or says what a resumed pass does with a half-deleted version.
    **Blocks**: `cpt-cf-bss-products-dod-retention-order`,
    `cpt-cf-bss-products-dod-restore-drill`.
    **Owner**: this feature with `06-catalog-version`, which owns the predicate the order spends.

26. **What `aggregate_id` do `ActorErased` and `PiiAllowlistChanged` partition on?**
    `infra::events::enqueue` requires one and **P-D-22** fixes
    `partition = hash(tenant_id, aggregate_id) mod N`. An erased actor and an allow-list entry are
    not aggregates in that sense. `features/catalog-version.md` §7 rows 27 and 47 reach the body core
    and the subject type and **neither reaches the partition key**.
    **Blocks**: `cpt-cf-bss-products-dod-retention-events`.
    **Owner**: the events/audit owner with the PRD §4.5 owner.

27. **Is the audit-class retention window config or a DDL constant?** `design/10` §1.8 calls the
    retention durations config; `m20260829_000004` says *"A trigger cannot read configuration, so
    there is no predicate to write yet"* and plans the arm as a literal `OLD.written_at < <cutoff>`.
    If it is config the trigger cannot enforce it and the arm must admit any authorised `DELETE`; if
    it is a DDL constant an operator cannot set it per jurisdiction, which PRD §15 says Legal and
    Finance will supply.
    **Blocks**: `cpt-cf-bss-products-dod-retention-clock`.
    **Owner**: `01-foundation`'s owner, whose migration holds the guard, with Legal and Finance.

28. **Three "configured" operands have no home.** `ProductsConfig` ships exactly two fields —
    `idempotency_retention_hours` and `require_broker` — and neither the pseudonymization age, the
    drill cadence nor the retention durations is among them. PRD §15 marks the values themselves
    **TBD**, owned by Legal and Finance. `features/reference-signal.md` met the same gap by obliging
    four config fields in a DoD; this feature does not, because the values' owner is outside the
    gear.
    **Blocks**: `cpt-cf-bss-products-dod-erasure-age`,
    `cpt-cf-bss-products-dod-restore-drill`, `cpt-cf-bss-products-dod-retention-clock`.
    **Owner**: this gear's config owner with Legal and Finance.

29. ~~**Which tables hold break-glass sessions and correction overrides for the clock?**~~
    **Closed by measurement (2026-09-03, no decision taken): each has a store of its own.**
    `m20260901_000017_create_products_breakglass_session.rs` creates `products_breakglass_session`
    carrying `opened_at`, and `m20260901_000022_create_products_correction_override.rs` creates
    `products_correction_override` carrying `recorded_at`. Neither rides `products_audit_log`, so the
    DoD's enumeration is a **table** list as written and each store hands the clock its own timestamp
    column to sweep — which is what that DoD's **Touches** block already names. Original text:
    `cpt-cf-bss-products-dod-retention-clock` puts both at statutory maximum, and no document names
    either table in this feature's scope: `design/10` §4 names only the map and the allow-list.
    Whether they ride `products_audit_log` — making the DoD's enumeration a class list rather than a
    table list — or each has a store of its own, is unstated.
    **Blocks**: no DoD — **closed by measurement**; `cpt-cf-bss-products-dod-retention-clock` stays
    blocked by rows 16, 27 and 28.
    **Owner**: was `05-governance` (break-glass) and `07-reference-signal` (correction overrides)
    with this feature; **discharged**.

30. **Does this feature own the DR posture, and which DoD delivers it?** §1.2's divided-requirement
    table claims *"restore verification and the DR posture"*, §1.1 promises storage class and DR
    posture, and **no DoD names DR config or a DR probe**. Row 10 asks the question in the slice's
    terms and does not reach this document's own claim.
    **Blocks**: no DoD — it decides whether a thirteenth is owed or §1.1 and §1.2 narrow.
    **Owner**: the design owner with the NFR #5 workshop — row 10's owner.

31. **Does `cpt-cf-bss-products-dod-pii-allowlist` owe a column roster?** It is the only DoD here
    that creates a table and it names no column, while `cpt-cf-bss-products-dod-identity-map`
    restates ten column and guard names for a table that already ships. No document names a column of
    `products_pii_allowlist`. Either §1.4's restatement rule is narrowed to tables with a shipped
    schema, or `design/10` §4 is extended and this DoD restates it.
    **Blocks**: no DoD directly — it decides whether one is incomplete.
    **Owner**: the design owner with `05-governance`'s, which owns the `GovernedLiveOp` shape.

32. **Is §6 owed one criterion per DoD?** Five DoDs have no criterion, among them the authz DoD whose
    own body argues that unnamed obligations are ticked by inspection. The general question is
    `features/catalog-version.md` §7 row 50's; this row records that it bites here too.
    **Blocks**: no DoD; it decides whether §6 is short.
    **Owner**: the design-set owner, as that row names.

### Owed to other documents, recorded and deliberately not edited

- **`features/foundation.md` §1.4** claims `artifacts.toml` excludes design slices from
  autodetection. It is false — `[systems.autodetect.artifacts.DESIGN_SLICE]` carries
  `pattern = "design/*.md"` and `traceability = "FULL"`. It is the shape donor for the four unwritten
  FEATUREs, so the cost compounds.
- **`repo::AuditCommon::correlation_id`**'s owed migration — see row 16. Owner: `01-foundation`.
