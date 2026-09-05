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

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-identity-map`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| a second, **tombstone-inclusive** read, for the export alone | `repo::identity_entries_of_principal` | `the_two_reads_keep_their_separate_predicates`, which asserts the *absence* of the shipped resolve's filter rather than the presence of a function |
| the erasure door **MUST NOT** call the minting resolve | `repo::tombstone_principal` carries its own | the same case scans this module's code — comments stripped, because the module's doc explains the rule at length and the first scan reddened on the explanation |
| this DoD creates **no table** | none added; the index rides the allow-list's migration | `m20260901_000028`'s own doc, and **P-D-118** item 18 routed it there on the *"an index rides the change that makes its read live"* rule |
| the read-by-principal path has a covering index for the tombstoned case | `idx_products_identity_ref_principal_tombstone`, **total** and not partial | `postgres_retention_schema::the_allowlist_unique_is_partial_and_the_tombstone_index_is_not` reads both `indexdef`s as text — a partial one here would be the covering index that already exists |
| `last_seen_at` advances on a **resolve** and not on a mint | `repo.rs`'s one `LastSeenAt` write, inside `resolve_actor_ref` | `repo_tests`' shipped pair, plus `one_writer_advances_last_seen_at_and_one_door_reaches_it` |
| **every stamping door** honours it | there is exactly one stamping path: one writer, reached by one shared actor context | the same census. *"Every door"* is a claim about a **set** that no behavioural probe can close; a singleton is the property that makes it true, and the assertion fails the day a second writer appears — at which point the clause needs a real census and this is the thing that says so |

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

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-erasure-age`

The system **MUST** run the **same tombstone act** automatically at the configured maximum age,
emitting the **same** event. Erasure-on-request and erasure-on-age are **one mechanism with two
triggers**, and this DoD **MUST NOT** produce a second code path.

The age operand **MUST** be `last_seen_at` — the age of the principal's **last activity in the
tenant** — and **MUST NOT** be `first_seen_at`. Age-since-first-appearance would tombstone an active
employee mid-employment, which is the failure the column's semantics were corrected to prevent.

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| runs **automatically** at the configured maximum age | `infra::retention::tombstone_aged_principals`, a call in `gear.rs`'s loop | `the_age_trigger_tombstones_the_aged_principal_and_only_that_one`, which drives the function with no request anywhere |
| the **same tombstone act** | it calls `repo::tombstone_principal` — the door's own function, not a copy | `the_age_path_writes_the_same_map_state_and_announces_the_same_event` asserts the map state the door leaves |
| the **same** event | `enqueue_retention` with `ACTOR_ERASED_PAYLOAD_TYPE` and `act: "erased"`, the door's own arguments | the same case counts exactly one `ActorErased` |
| **MUST NOT** produce a second code path | there is one `tombstone_principal` and one `ActorErased` emitter shape; this module reaches both exactly as the door does | the two cases above, plus `dod-identity-map`'s singleton census one layer down |
| the operand is `last_seen_at`, **never** `first_seen_at` | `repo::principals_older_than` filters on `LastSeenAt` | the same case seeds a principal minted 900 days ago and one minted 10 days ago and asserts **only** the first is tombstoned — the negative half, which a sweep that tombstoned everything would fail |
| under the system principal, with the age rule's own reason | `gear::system_actor_ref()` and `AGE_REASON` (**P-D-117** item 14) | `the_age_path_…` asserts the row's reason names `inst-er-age`: no human supplied one, and a row without one would be a hole in the class this feature retains |

**The cadence cannot double-erase.** `principals_older_than` excludes tombstoned rows and
`tombstone_principal` re-asserts `tombstoned_at IS NULL` in its own `UPDATE`, so a second pass
neither restamps a column the entity's doc pins as *"set once … and never cleared"* nor announces a
second erasure — `a_second_age_pass_neither_restamps_nor_re_announces`, both halves.

**Implements**: `cpt-cf-bss-products-flow-erasure`

**Touches**:
- DB Table: `products_identity_ref`, `products_audit_log`
- Entities: `IdentityRefMap`, `RetentionClock`

### The compliance export

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-compliance-export`

The system **MUST** serve `GET /bss-products/v1/compliance/identity-export` on **`compliance ×
export`** — **its own grant, never `audit × export`** — returning, per named principal, that
principal's map entries plus the audit-row references carrying their refs, DSAR-shaped. **Every
access MUST be individually audited.**

The separate grant is not stylistic: `design/10` §4 excludes the map from `audit × export`'s output, and this is
the one surface that returns **real identities**. Folding it into the audit grant would hand every
auditor the identities the whole pseudonymization scheme exists to withhold.

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| the route on **`compliance × export`**, never `audit × export` | `api::rest::retention`'s router, `Gate::Compliance` | `the_export_returns_the_tombstone_and_audits_the_access`; the gate's `action()` reads `EXPORT` off the `COMPLIANCE` resource and nothing else spends it |
| per named principal, that principal's map entries | `repo::identity_entries_of_principal`, the tombstone-inclusive read | `the_two_reads_keep_their_separate_predicates` asserts the export's read carries **no** `tombstoned_at IS NULL` filter — the predicate that would answer a post-erasure DSAR *"no entries"* |
| plus the audit-row references carrying their refs | `repo::audit_refs_of_actors`, matching **either** column | the shipped case; an erased principal never *acts* in the row recording its erasure, so an `actor_ref`-only match hid every erasure from the DSAR that asked about it |
| **every access individually audited** | `repo::write_audited_read_audit`, in its own transaction **before** anything is served | `three_exports_write_three_access_rows` — a count, because a probe that reads *an* audit row cannot tell one write from three |
| a **justification** is required (**P-D-133**) | `ExportQuery::justification`, refused blank, then through the write block | `the_export_requires_a_justification_and_records_it_on_the_access_row`, **both halves**: refused without it, and the value asserted **on the row** — a door that demanded the field and dropped it passes the refusal half alone |

**The holder is not named here, and that is the DoD honoured rather than deferred.** §7 item 8's
remaining half — which principals hold `compliance × export` — is Architecture's with Legal, and
the door checks the grant and nothing else (**P-D-133**). Under **P-D-109** that question defeats
no clause above: it asks *who may be given* the grant, and every obligation here is about what the
door does for whoever holds it.

**Implements**: `cpt-cf-bss-products-flow-erasure`

**Touches**:
- API: `GET /bss-products/v1/compliance/identity-export`
- DB Table: `products_identity_ref`, `products_audit_log`
- Entities: `IdentityRefMap`

### The PII detector policy

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-detector`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| block / allow / allow-by-list | `domain::retention::RegistryPiiDetector::inspect` | `the_four_arms_each_have_a_positive_control` — **every arm paired with a control**, including the near-misses the two blocking arms exist to not catch (`ops@ the desk`, `SKU-1234567`) |
| **uncertainty blocks** (C2) | left in `taxonomy::content_pii_block`, untouched | `an_unlisted_name_is_uncertain_and_an_address_is_blocked` asserts both verdicts **and** that both refuse at the hook |
| `CONTENT_PII_BLOCKED` naming the field | the hook's own rendering | `a_pii_refusal_names_the_field_and_its_audit_row_carries_no_detected_value` |
| **never the detected value** | every reason names a *shape*; the candidate is used to consult the list and dropped | `no_verdict_reason_carries_the_matched_text` sweeps **every** blocking arm and checks the hook's rendering too — a one-case probe would not see an arm added later with the match interpolated in. And the audit row is asserted clear, because the row is the record erasure cannot rewrite |
| the hook is `02`'s and is not relocated | nothing in `domain::taxonomy` changed | the same file's own tests stand unedited |
| **the whole door set** raises the same code | all six production sites now build this detector | `no_production_door_builds_the_permissive_pii_host`, with `the_permissive_host_census_can_fail` as its perturbation |
| mint no second `DomainError` arm | none added | `dod-retention-error-taxonomy`'s roster stays at one owned code |

**The door set was six sites, not two.** Measured 2026-09-04: `NoPiiPolicyDetector` was constructed
at **six** production sites, each its own `Arc::new(..)` literal — `products.rs` (`save_product`),
`skus.rs` (`save_sku_gated`), `taxonomy.rs` (`label_operand` and `merge_metadata`),
`materiality_policy.rs` (`set_materiality_policy`) and `retention.rs` (`execute_erasure`) — so
*"the registered detector"* named a phrase and not a registry, and swapping "the call site" would
have left four doors admitting every string while their neighbours refused. All six now call
`api::rest::retention::tenant_pii_detector`, and the census above is what keeps a seventh from
arriving with its own literal.

**What makes this detector uncertain, stated** (P-D-117 deliberately left the policy to this
slice). **It cannot tell a person's name from a product named after one** — and that is not a gap
a better heuristic closes, it is the exact question the allow-list exists to have a human answer on
paper. So an unlisted person-shaped run is `Uncertain` and never `Blocked`: `Blocked` asserts a
finding — *this is personal data* — that nothing here established, and that false assertion would
reach the operator's refusal and its audit row. `Uncertain` says the true thing, the write is still
refused because the hook holds C2, and the refusal points at the lane out. Email addresses and
telephone numbers are `Blocked`, for the mirror reason: there the shape **is** the finding, and
calling it uncertain would understate what the detector knows.

**A defect this DoD's own probe caught.** The phone arm was written per whitespace-separated token,
so `+44 20 7946 0958` — four tokens of two to four digits — matched nothing and the arm blocked no
number at all. It is now a whole-text scan over runs of digits and separators, floored at nine
digits so a catalog identifier is not a phone number, capped at E.164's fifteen, and requiring
every digit group to be at least two long so `tiers 1 2 3 4 5 6 7 8 9` is an enumeration rather
than a number.

**Implements**: `cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- Entities: `PiiDetector`

### The Legal-governed allow-list

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-allowlist`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| create `products_pii_allowlist` on P-D-117's roster | `m20260901_000028_create_products_pii_allowlist.rs`, both engines | `postgres_retention_schema::the_allowlist_roster_matches_on_postgres`, transcribing the roster from the **decision** so a column dropped from code and migration together is still red |
| **mandatory** Legal sign-off reference | `signed_off_by NOT NULL` plus the door's own check | `a_missing_sign_off_reference_is_refused_by_field_and_a_complete_entry_is_admitted`, **with its positive control in the same case** so the rule cannot be proven by a door that admits nothing |
| refused riding `01`'s `VALIDATION`, the violation naming the field (**P-D-64**) | `sign_off_allowlist_entry`'s `report.violate("VALIDATION", "signedOffBy", …)` | the same case, which asserts the violation's `type` **and** its `subject` — the code alone would pass on a violation naming the wrong field |
| per tenant | PK `(tenant_id, entry_id)`, every read filtered on `tenant_id` | the roster probe |
| audited | `write_allowlist_audit`, in the act's transaction | `each_allowlist_act_writes_one_audit_row_and_one_event`, counted at **1** per act |
| exportable for the Legal review | `GET /bss-products/v1/compliance/pii-allowlist` → `repo::allowlist_entries` | `a_revocation_keeps_the_row_and_its_sign_off_in_the_review` |
| a `GovernedLiveOp` on `pii_allowlist × write` under the base quorum (**P-D-10**) | `submit_allowlist_to_gate`, at both mutating doors | `both_allowlist_doors_submit_their_act_to_the_gate` — **call sites, not a verdict**: the registered host authorizes everything, so a green verdict would prove nothing about whether the ceremony was asked |
| emit `PiiAllowlistChanged` | `emit_allowlist_changed`, same transaction | `each_allowlist_act_writes_one_audit_row_and_one_event` asserts **2** after both acts: a revocation nobody hears leaves a stale cache admitting a withdrawn name |

**Revocation is a state flip and never a `DELETE`** (P-D-47's reasoning one table over), and the
uniqueness is therefore **partial**: `UNIQUE (tenant_id, value_normalized) WHERE state = 'active'`.
Both arms are probed on both engines —
`the_active_uniqueness_is_partial_and_a_revoked_value_may_be_signed_off_again` and
`postgres_retention_schema::a_second_active_value_is_refused_on_postgres_and_admitted_after_a_revoke`
— because a **total** `UNIQUE` passes the refusal half and fails the re-sign-off half, and no index
at all does the reverse. The Postgres probe reads the index's `indexdef` as **text**: an index
created without its `WHERE` exists under the right name and enforces the wrong rule.

**The match rule is exact equality on the normalized value, and the normalization is the whole of
the rule** (P-D-117 item 23). `domain::retention::normalize_allowlist_value` is its one
implementation — **NFKC**, then trim and collapse internal whitespace runs, then lowercase — and
**both sides run through it**, the stored column and the detector's own candidate, so the equality
has one definition rather than two that can drift. It deliberately does **not** strip punctuation,
drop diacritics or reorder words; each would widen the match past what the sign-off covered, and
those limits are asserted as inequalities in `the_normalization_does_not_widen_past_the_sign_off`
because a rule's limits are what a later "improvement" silently removes.

**Implements**: `cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- DB Table: `products_pii_allowlist`, `products_approval`
- Entities: `PiiDetector`

### The retention clocks

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-clock`

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

#### The guard shape `07` row 38 routes here — proposed, not built

**P-D-129**'s recommendation is *"one shape for the whole class, the audit plane's row-image
predicate (P-D-34)"*. **The audit plane has no such predicate**, measured at `951fd3bae`:
`m20260829_000004` refuses **every** `DELETE` unconditionally on both engines — the same flat
refusal as `products_approval`, `products_approval_decision`, `products_breakglass_session` and
`products_correction_override` — and its own doc says, citing **P-D-118**, that *"there never will
be"* one: *"No `OLD.written_at < <cutoff>` arm is written in this file … the DELETE arm this
trigger admits is the GC's identity, not a date."* **P-D-31** then makes that identity unreadable,
and the same file says so: *"the session variable that would carry it exists on Postgres and not on
`SQLite`, so neither trigger reads one."* The chain's only **opened** predicate is
`m20260829_000007`'s, and it is **referential** (`WHERE EXISTS` against
`products_catalog_version_entry`, P-D-40) rather than row-image — it reads another table, not
`OLD`'s own columns.

So the recommended shape does not exist and the document that would host it forbids it, and the
reason generalises: **a trigger can express properties of the row, never properties of the
deleter.** P-D-31 removed the only identity channel and P-D-118 removed the date; nothing is left
for a predicate to read that separates the GC from any other caller. *"A predicate the GC's own
preceding admitted write can make true"* needs an admitted write, and `products_correction_override`
admits **no `UPDATE` at all** — *"Evidential rows admit no `UPDATE` and no `DELETE` … there is no
admitted edit at all"* — so the shape cannot be built there without opening a write path on the
table whose doc exists to refuse one; and a claim column any caller may set is a two-statement
speed bump rather than a guard.

**The proposal is therefore to stop authorising the GC in DDL and fix the failure the row actually
describes** — *"a collector reaching a statutory-max row on a flat refusal raises `P0001` … so the
sweep aborts and takes its other candidates with it"*. Three parts: **(a)** the sweep enumerates and
deletes **per class, per candidate, each in its own transaction**, which P-D-118 item 25 already
forces for catalog versions; **(b)** a class whose table refuses `DELETE` yields a **held**
candidate carrying a named reason — the `retention_orphan_blocked` shape `dod-retention-gate`
already ships — reported and never retried into an abort; **(c)** the five migrations are **left
untouched**, because on this reading `products_approval`, `products_breakglass_session` and
`products_correction_override` are *evidence* whose flat refusal is the correct guard rather than a
hole, and at `retention_days_audit` = 3650 interim no collector reaches one of their rows for ten
years. If that is right, row 38 closes as *"the shape is the application's, not the DDL's"*. If the
owner wants those rows deletable, that is a decision to open a write path on evidence and should be
taken as one. **No migration is edited under this DoD until it is taken.**

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| candidates **per record class**, each at the statutory maximum | `infra::retention::sweep_class`, one arm per `domain::retention::RecordClass` | `each_class_reads_its_own_window` narrows **one** window and asserts the other two classes' candidate counts are **unchanged** — the half a single-class probe cannot see |
| never *"indefinite"* (C3) | every class reads a configured window; there is no unbounded arm | the same case, which reaches a candidate at all only because a window exists |
| across frozen versions, catalog versions, audit rows and the four evidential stores | `repo::catalog_version_candidates`, `entity_version_candidates`, `audit_class_candidates` (five stores in one function, one clock column each) | `every_pass_writes_an_audit_row_carrying_its_class_clock_and_verdict` reaches all three classes |
| the audit class deletes **whole rows**, never a refusal without its classifier | `repo::delete_audit_class_row` deletes by primary key, no column-level write | the guard refuses it whole, which is what `a_refusing_class_is_held_and_the_others_still_collect` reads |
| **two populations outside every clock** — the outbox (P-D-22) and `07`'s operational tables | no candidate read reaches either | `the_excluded_populations_are_never_candidates`, a source census over the store layer **plus** a zero-window control so the absence is not proven by a sweep that sees nothing |
| the window is the **GC's** predicate, never a trigger arm | `RetentionCaps::cutoff`, read from configuration | `m20260829_000004`'s doc, which P-D-118 made normative, and no `OLD.written_at` arm exists anywhere |
| every GC act audited with the class, the clock and the gate verdict | `infra::retention::write_pass_audit` | `every_pass_writes_an_audit_row_carrying_its_class_clock_and_verdict` reads `class=`, `cutoff=` and `held_reason=` off the row |

**Which table is in which class, and the evidence.** `PRD` §15 names three windows and no table, so
the mapping is read from the two documents that do name one. **Financial** is the catalog-version
chain, on `PRD` §330: *"**Snapshots are financial records**: `CatalogVersion` snapshots + version
history …"*. **Version** is `products_entity_version`, kept separate rather than folded in, because
`dod-retention-order` says version-row retention *"**derives** from catalog-version retention and is
**never shorter**"* — a sentence that is vacuous if the two share one window and load-bearing if
they do not. **Audit** is the audit log and the four evidential stores, on this DoD's own words:
*"all audit-grade"*. The three interim numbers are equal, so nothing distinguishes them in
behaviour today; the mapping is still a choice and is stated in `domain::retention::RecordClass`
rather than left to whichever constant a call site reached for.

**Three of the four target tables refuse every `DELETE` at this commit, measured.** The only opened
predicate in the chain is `products_entity_version`'s (`m20260829_000007`, **P-D-40**).
`products_catalog_version` (`m20260901_000010`) refuses outright with no note;
`products_catalog_version_entry` and `_capture` (`m20260901_000013`) refuse with an interim message
naming *"slice 10's manifest retention"* — this feature — as their future admitter; and the five
evidence tables are **P-D-136**'s decided posture. So the sweep's steady state is: the version class
collects rows no manifest references, and **every other class is held**. That is reported rather
than assumed — the sweep offers each delete and classifies the refusal — so the day a migration
opens an arm the sweep starts collecting with no edit here. **The catalog-version chain's two
migrations are outside this assignment's grant and P-D-136 does not mention them**; whether they
join the evidence class or get their arms opened is recorded in §7 as item 34.

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

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-order`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| capture and entry rows **before** their catalog-version row | `repo::delete_catalog_version`, three statements in that order in **one** transaction | `a_held_catalog_version_keeps_its_entries` — the refused delete rolls back **whole**, and an entry lost on the way to a refused manifest row is exactly the surviving-manifest state P-D-118 item 25 forbids |
| entity versions only **after** every referencing manifest | not ordered by this module at all: each candidate is offered to `m20260829_000007`'s referential predicate | `a_referenced_version_is_refused_by_the_guard_and_not_by_the_sweep`, whose second half issues the `DELETE` **directly, with the sweep bypassed** — §6's own words |
| the derive rule: retention is **never shorter** | the same predicate; a referenced row is `HeldReason::ReferencedByRetainedManifest` | the first half of that case: two candidates, the orphan collects and the referenced one holds |
| every GC act audited with the class, the clock and the gate verdict | `write_pass_audit` | `every_pass_writes_an_audit_row_carrying_its_class_clock_and_verdict` |

**The ordering is deliberately not this module's guarantee.** `repo::entity_version_candidates`
does **not** pre-filter by manifest reference, and that is the criterion honoured rather than an
omission: a sweep that filtered first would move the guarantee from the engine to the sweep, and
§6's criterion refuses it by name — *"refused **by the guard**, not merely skipped by the GC — the
probe passes even when the GC is bypassed entirely"*. So the sweep offers every candidate and
reports what the engine decided.

**Implements**: `cpt-cf-bss-products-flow-retention`

**Touches**:
- DB Table: `products_entity_version`, `products_catalog_version_entry`, `products_catalog_version`
- Entities: `RetentionClock`, `RetentionGate`

### The restore drill

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-restore-drill`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| on the configured cadence | `gear::drill_due(tick_count, drill_cadence_hours)`, the third loop call | the predicate is a function of the **configured** hours, not a constant beside them |
| a sampled set of catalog versions **and their referenced entity versions** | `repo::newest_catalog_versions` then, per version, `catalog_version_manifest_rows` and `entity_version_digest` per entry | `a_clean_restore_verifies_both_halves` asserts `verified=2` — one manifest **and** one referenced row |
| from **backup, into an isolated target** | `drill_target_dsn`, opened as its own `DBProvider`; nothing writes to it | the drill's reads are `find`/`all` only, and the run's own audit row goes to the **live** database |
| re-verify **both** the manifest checksums and each row's `content_digest` | `VersionManifest::checksum()` rebuilt from the restored rows, and `canonical::content_digest(&row.content)` | the same case; manifest checksums alone are blind to version-history corruption, which is why both counts are asserted |
| **byte-for-byte** (C5) | `Vec<u8> == Vec<u8>`, no rendering in between | `a_corrupted_restore_raises_the_alarm` rots the stored digest and reads `corrupt=1` |
| compare **like with like** on `digest_version` | the version is checked **before** the digest is compared, on both halves | `a_foreign_digest_version_is_unverifiable_and_not_corruption` seeds a foreign version **and** a bad digest, and asserts `unverifiable=1 corrupt=0` — a drill that re-rendered would manufacture the mismatch P-D-133 item 7 forbids |
| **MUST NOT** expect the four moving columns | the recomputation's only operand is the stored `content` | `the_drill_recomputes_over_the_stored_content_and_nothing_else`, a source census: a column outside that string cannot reach the comparison however it moves |
| a byte mismatch is a **compliance incident alarm**, not a log line | `products_restore_drill_corruption` at `error!`, and the count on the run's audit row | the corruption case reads the **row**, since §6 says *"in the result, not only in a log line"* |
| results on an operator surface with the **last-verified watermark per tenant** | one audit row per run (**P-D-134** item 6); the watermark is the newest such row per tenant — a query, not a table | every drill case reads it back that way, which is the surface existing |

**The sample is the newest twenty catalog versions, and every row inside them is scanned.** Newest
rather than random for two reasons, both about what a drill is for: corruption is found by reading
the restore an incident would actually restore from, and a deterministic sample makes two
consecutive runs comparable where a random one turns a regression into a coin flip. P-D-133 item 7's
*"every sampled row is scanned on every drill"* is then exact — the sample bounds the **versions**
looked at and never the rows verified within them.

**With no target configured the run still happens** (**P-D-135**): it writes its audit row with
outcome `no_target` and raises `products_restore_drill_unverifiable`. A target that is configured
and will not open is a **different** outcome, `unreachable`, because the two need different
operators — one is a deployment that has not wired the drill, the other is a restore that is not
there. `an_unconfigured_drill_still_records_its_run` asserts the row exists and claims nothing it
did not check.

**No metrics facility was added.** The alert channel is `tracing::warn!` / `error!` with the stable
event names, as `gear.rs`'s loops already do. The toolkit this gear links exposes none, and adding a
crate for one is a dependency decision rather than this `DoD`'s — reported, as the brief asked.

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

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-retention-events`

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

**Built, clause by clause, with the call site named for each** (P-D-109's discipline; ticked
2026-09-04).

| clause | where it lands | what proves it |
|---|---|---|
| emit `ActorErased` | `api::rest::retention::execute_erasure`, inside the tombstone's transaction | `an_erasure_announces_itself_and_a_refused_one_does_not` — and the **negative** half is the one that matters: a refusal that still announced would tell every cache to drop a ref that was never retired |
| emit `PiiAllowlistChanged` | `emit_allowlist_changed`, at both allow-list doors | `each_allowlist_act_writes_one_audit_row_and_one_event`, **2** after both acts |
| each **in the same transaction as its act** | both ride the caller's `tx`; an enqueue failure is `TxError::Events` and rolls the act back | the refused-erasure half above |
| each carrying a **versioned** schema reference | `SCHEMA_REFS` gains `bss-products.ActorErased.v1.0.0` and `…PiiAllowlistChanged.v1.0.0` | `the_schema_roster_names_exactly_the_declared_events`, both directions, and `every_schema_reference_is_semver_and_names_its_own_event` |
| **both** rosters extended, neither derived from the code | `events_tests::THE_RETENTION_PAIR` and `broker_tests::THE_RETENTION_PAIR` | `each_event_declares_its_derived_type_id_and_subject_type` and `every_event_reaches_the_broker_under_its_own_type_id` |
| the aggregates (**P-D-118** item 26) | `ActorErased` on the erased `principal_ref` via `events::retention_aggregate_id`; `PiiAllowlistChanged` on `entry_id` | `enqueue_retention`'s own doc and the door call sites |
| `ActorErased` carries the ref and **no identity** | `RetentionEventPayload` has no field an identity could reach | the payload's own shape, and the broker roster's `actorRef` assertion which the field naming had to obey |

**"Both `THE_EIGHT` rosters" is satisfied as two new lists, not two extended ones**, and that is the
rule those files enforce rather than a deviation from this sentence. `events_tests` carries six
transcribed rosters — §4.5's eight, `04`'s pair, `04`'s rest, `03`'s trio, `09`'s summary, `02`'s
eight — each its own for one stated reason: *folding a slice's events into a neighbour's list makes
that neighbour's own completeness claim uncountable*. `THE_EIGHT` is a transcription of one
sentence in `01` §4.5, and adding `ActorErased` to it would claim §4.5 announces it. The obligation
the DoD's sentence carries is that **both files' roster sets grow in this change**, and they do.

**`actorRef` is the acting principal, and the erased pseudonym is `erasedActorRef`** — a rename a
probe forced. Every typed event's payload carries `actorRef` as P-D-01's *acting* principal, and
`broker_tests` asserts it across the whole roster; naming the erased subject `actorRef` would have
collided with a field every consumer already reads as *"who did this"*.

**Implements**: `cpt-cf-bss-products-flow-erasure`,
`cpt-cf-bss-products-flow-pii-policy`

**Touches**:
- Entities: `IdentityRefMap`, `PiiDetector`

## 6. Acceptance Criteria

**34 of 34 ticked, each against a named probe** (the last at P-D-158; the status box waits on the studio's begin/end code markers for the six §2–§3 items and the DECOMPOSITION entry — P-D-158 routes that to the close-out group) (**P-D-137**'s convention: a criterion ticks with
its `DoD`, by the strand, clause by clause, and never by inspection — so every ticked line below
carries the case that proves it). The one that is not ticked is the detector's enumerated door set:
four of the five doors it names are `07`'s and `04`'s and do not exist yet. **That is why this
feature's status box is still unticked**, and it is a finding rather than an omission — see the
criterion's own note.

**The reproducibility-vs-erasure flagship**

- [x] Freeze a version, erase its approver, then assert **both halves in one probe**: the old
      snapshot's checksum is **unchanged**, and the rendered audit shows the **tombstone**. C1 is
      only proven by asserting both — either half alone passes on a build that got the other wrong.
      *Probe:* an_erasure_leaves_a_frozen_record_byte_identical.

**The detector**

- [x] A matrix of block / allow / allow-by-list / uncertainty-blocks, **each with a positive
      control**, so no arm passes because the fixture could not reach the permissive branch.
      *Probe:* the_four_arms_each_have_a_positive_control.
- [x] A block names the **field** and the assertion checks the refusal does **not** carry the
      detected value.
      *Probe:* no_verdict_reason_carries_the_matched_text,
      a_pii_refusal_names_the_field_and_its_audit_row_carries_no_detected_value.
- [x] Every `reason`-bearing door in the enumerated set raises the same code: audit rows, approval
      rejections, break-glass sessions, correction overrides, and the `SkuRetired` payload — and
      **not** bulk or promotion rows, which P-D-50 struck.
      **No probe, and this is the one criterion that keeps the feature's status box
      unticked.** Re-measured 2026-09-05 (P-D-152): the hook runs at **fourteen** production
      sites and every door this criterion enumerates now exists and calls it — approval
      rejections and break-glass sessions (`approvals.rs`), correction overrides (`skus.rs`),
      the retirement reason (`products.rs`, `skus.rs`) — but door-level probes of the code exist
      at three doors only (`products_tests`, `retention_tests`, `reference_tests`), so the
      criterion stays unticked by P-D-137's rule: a placement is verified by its call site, a
      criterion by its probe. Measured 2026-09-04: the hook runs at **seven** production doors, and
      four of the five this criterion enumerates have no door to run it at yet —
      `products_correction_override`'s reason belongs to `07`'s unbuilt correction door,
      and the `SkuRetired` payload's to `04`. Two of the five now exist and are covered:
      the audit-row reasons (every door in this feature) and the approval rejection and
      break-glass reasons (`api::rest::approvals`, whose site this commit swapped off the
      permissive host). A criterion cannot be ticked against doors that do not exist, and
      ticking it by inspection is what P-D-137's convention forbids.
      **Ticked (P-D-158, 2026-09-05), one probe per enumerated door**, each a person-shaped reason
      meeting `CONTENT_PII_BLOCKED` at the wire with nothing written: approval rejections
      (`a_rejection_with_a_person_shaped_reason_is_refused_content_pii_blocked`), break-glass
      sessions (`an_elevation_with_a_person_shaped_reason_is_refused_content_pii_blocked`),
      correction overrides (`break_glass_arm_a_needs_the_flag_every_producer_stale_and_a_clean_reason`,
      now naming the code), the `SkuRetired` reason on both retire doors
      (`a_retirement_reason_naming_a_person_is_refused_content_pii_blocked`,
      `a_product_retirement_reason_naming_a_person_is_refused_content_pii_blocked`), the
      materiality-policy reason (`a_policy_reason_naming_a_person_is_refused_content_pii_blocked`),
      the audit-row reasons (`a_pii_refusal_names_the_field_and_its_audit_row_carries_no_detected_value`).
      Bulk and promotion rows carry no reason field (`the_row_shape_is_judged_in_one_report`), so
      the hook has no operand there — P-D-50's strike holds by shape.

**The allow-list**

- [x] A mutation runs the **base quorum** and is refused when the Legal sign-off reference is
      absent, **asserted with its positive control** — a mandatory-field rule proven only by its
      refusal is a rule that may never admit anything.
      *Probe:*
      a_missing_sign_off_reference_is_refused_by_field_and_a_complete_entry_is_admitted,
      both_allowlist_doors_submit_their_act_to_the_gate.
- [x] An admitted entry is exportable for the Legal review.
      *Probe:* a_revocation_keeps_the_row_and_its_sign_off_in_the_review.

**The retention gate, RED first**

- [x] A candidate version with one live freeze-registration is **skipped and alarmed**.
      *Probe:* a_freeze_held_catalog_version_keeps_its_entries.
- [x] The same version GCs cleanly once **every** registration satisfies the pair — `state =
      released`, or `state = not_frozen(forced)` with `released_at` stamped.
      *Probe:* a_released_catalog_version_collects_whole.
- [x] A registration at `state = acked` **beside a stamped `released_at`** is **still skipped and
      alarmed**. This is the correction's own regression probe: reading the timestamp alone collects
      a version holding live grandfathered references.
      *Probe:* domain::retention_tests' recovered-forced-participant case.
- [x] An **empty** `participant_set_snapshot` is **collectable**; an empty **registration ledger**
      under a non-empty snapshot is **not**. The two are asserted apart, because quantifying over the
      wrong one satisfies the gate vacuously.
      *Probe:* domain::retention_tests' two vacuity cases.

**Derived retention and order**

- [x] An entity-version row referenced only by a **retained** catalog version survives its own class
      clock.
      *Probe:* a_referenced_version_is_refused_by_the_guard_and_not_by_the_sweep.
- [x] A GC attempting an entity-version `DELETE` while a manifest entry still references it is
      refused **by the guard**, not merely skipped by the GC — the probe passes even when the GC is
      bypassed entirely.
      *Probe:* the same case's second half, with the sweep bypassed.

**The drill**

- [x] A deliberately **corrupted** backup sample fails the drill loudly. The oracle must be seen to
      fail.
      *Probe:* a_corrupted_restore_raises_the_alarm, controlled by
      a_clean_restore_verifies_both_halves.
- [x] A row written under an earlier `digest_version` produces a **version mismatch**, distinguished
      from a corruption alarm in the result, not only in a log line.
      *Probe:* a_foreign_digest_version_is_unverifiable_and_not_corruption.
- [x] The drill does **not** expect `lifecycle_state`, `deprecation_provenance`,
      `replaced_by_sku_id` or `internal_revision` in the digested content.
      *Probe:* the_drill_recomputes_over_the_stored_content_and_nothing_else.

**Erasure**

- [x] Age-based pseudonymization fires **without a request** and produces the same map state as the
      requested path. What "byte-identical in effect" covers beyond map state is §7's.
      *Probe:* the_age_trigger_tombstones_the_aged_principal_and_only_that_one,
      the_age_path_writes_the_same_map_state_and_announces_the_same_event.
- [x] A principal acting **after** its erasure mints a **fresh** ref rather than reviving the
      tombstoned row, and render-time joins of historical records still show the tombstone.
      *Probe:* the_export_read_sees_the_tombstone_and_a_later_mint_is_a_second_entry.
- [x] A repeat DSAR **after** an erasure still resolves by principal — which is what
      `principal_ref` surviving the tombstone is for.
      *Probe:* the same case, with
      an_erasure_destroys_a_seeded_payload_and_stamps_the_tombstone.
- [x] `last_seen_at` advances on a **resolve** and not on a mint, asserted directly, since the
      age trigger reads it.
      *Probe:* repo_tests' shipped pair,
      one_writer_advances_last_seen_at_and_one_door_reaches_it.

**Positive control, one line per declared code** — one code, one line.

- [x] `ERASURE_UNKNOWN_ACTOR` — a principal with no `actor_ref` in this tenant is refused **naming
      the principal**; the same request for a principal that has one succeeds.
      *Probe:* an_unknown_principal_is_refused_and_nothing_is_minted.

**The retention clocks** *(added 2026-09-04 under §7 item 32 — this DoD had no criterion)*

- [x] Each record class produces candidates at **its own** configured window, asserted by moving
      one window and watching only that class's candidate set change. A sweep that read one
      number for every class would pass a single-class probe.
      *Probe:* each_class_reads_its_own_window.
- [x] The two deliberately-excluded populations produce **no** candidates: outbox-delivered rows
      (the toolkit vacuum's horizon, **P-D-22**) and `07`'s watermark and member tables
      (operational current state). Asserted as absence, because an over-broad clock deletes
      records nobody asked it to.
      *Probe:* the_excluded_populations_are_never_candidates.
- [x] A class whose table refuses `DELETE` is reported **held**, and the sweep's other candidates
      still complete. The failure this guards is the one §7 row 38 describes: one `P0001` from a
      flat-refusal guard is not retryable contention, so an un-isolated sweep aborts and takes
      every unrelated candidate with it.
      *Probe:* a_refusing_class_is_held_and_the_others_still_collect, on the audit class.

**The compliance export** *(added 2026-09-04 under §7 item 32 — this DoD had no criterion)*

- [x] The door is refused without a **justification**, and the justification it demands is the one
      that lands on the access's audit row — **both halves**, because a door that required the
      field and then dropped it passes a refusal-only probe (**P-D-133**).
      *Probe:* the_export_requires_a_justification_and_records_it_on_the_access_row.
- [x] Three accesses write **three** audit rows. *Individually* audited is a count, and a probe
      that reads *an* audit row cannot tell one write from three.
      *Probe:* three_exports_write_three_access_rows.
- [x] The export spends `compliance × export` and the allow-list review spends it too; neither is
      served under `audit × export`.
      *Probe:* the_compliance_doors_spend_their_own_grant_and_never_the_audit_one.

**The two events** *(added 2026-09-04 under §7 item 32 — this DoD had no criterion)*

- [x] Each event is enqueued **inside its act's transaction**, asserted from the other side: a
      **refused** act enqueues none. An `ActorErased` beside a rolled-back tombstone tells every
      cache to drop a ref that is still live.
      *Probe:* an_erasure_announces_itself_and_a_refused_one_does_not.
- [x] Each carries a versioned schema reference and appears in **both** rosters — the interim
      arm's and the broker arm's — with the subject type its grant derives (**P-D-94**).
      *Probe:* the_schema_roster_names_exactly_the_declared_events,
      each_event_declares_its_derived_type_id_and_subject_type.
- [x] Neither payload has a field an identity could reach. `ActorErased` is a defensive
      cache-buster, and a field added later is how it would stop being one.
      *Probe:* neither_retention_payload_has_a_field_an_identity_could_reach.

**The authz surface** *(added 2026-09-04 under §7 item 32 — the DoD whose own body argues that
unnamed obligations are ticked by inspection)*

- [x] Each of the three grants is spent by a door that **exists**, and a caller without the grant
      is refused with the refusal **audited**. The four roster tests the DoD names are the
      declaration half; this is the spending half, and the DoD had only the first.
      *Probe:* each_grant_is_spent_by_a_door_and_a_denial_is_audited.

**The identity map's remainder** *(added 2026-09-04 under §7 item 32 — the DSAR criterion below
asserts the column, not the read)*

- [x] The tombstone-inclusive read returns a **tombstoned** entry, and the shipped
      `resolve_actor_ref` does not. The two differ by exactly the predicate that makes a DSAR
      after an erasure answer *"no entries"* — the one wrong answer that looks right.
      *Probe:* the_two_reads_keep_their_separate_predicates.
- [x] Exactly one writer advances `last_seen_at` and exactly one door reaches it. *"Every stamping
      door honours it"* is a claim about a set; a singleton is the property that makes it true,
      and the assertion is what fails the day a second writer appears.
      *Probe:* one_writer_advances_last_seen_at_and_one_door_reaches_it.

**Controls on the shipped seam**

- [x] `chk_products_identity_ref_tombstone` refuses a row carrying both a payload and a
      `tombstoned_at`, on **both** engines.
      *Probe:* a_row_with_both_tombstone_and_payload_is_refused_by_the_tombstone_check.
- [x] `uq_products_identity_ref_active` admits a second row for the same
      `(tenant_id, principal_ref)` **once the first is tombstoned**, and refuses it while the first
      is live. Both arms, because the partial predicate is the whole mechanism.
      *Probe:* a_second_live_ref_for_the_same_principal_is_refused_by_the_active_unique_index,
      a_fresh_ref_after_tombstoning_the_first_inserts_successfully.

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
   **Owner**: *(P-D-134, 2026-09-04: a recorded risk, not a question.)* this feature, operationally.

2. **Watermark/member tables (07) and bulk ledgers** carry `skuId`s and row payloads, not PII —
   asserted here so their retention rides ordinary classes; if a future producer's payload grew
   identity-bearing fields, the map discipline would apply — named to keep it from drifting in
   silently.
   **Blocks**: no DoD — it is an assertion recorded to stop a silent drift.
   **Owner**: *(P-D-134, 2026-09-04: a recorded fact, re-measured when a payload widens.)* this feature, with `07-reference-signal` and `09-bulk-promotion` if a payload widens.

3. **Encrypted-at-rest for the map** rides the platform storage posture; if a deployment lacks it,
   this table is the one that must not ship — a deployment gate, not a code path.
   **Blocks**: no DoD — it is a deployment gate.
   **Owner**: *(P-D-133, 2026-09-04: a mandatory item of the deployment checklist, the platform storage owner's.)* the platform storage owner.

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

5. ~~**What `actor_ref` attributes an unattended act's audit row?**~~ **Answered by P-D-113 arm 2 (2026-09-03): `gear::system_actor_ref()`, a stable UUID v5 from `bss-products:system`.** The premise — *no document names a system ref* — was true when written; the v4-per-boot it replaced was a defect. The age-triggered tombstone and
   every GC act are audited, 01 makes the audit row's `actor_ref` non-nullable, and every ref in the
   set is minted for a human principal on first appearance. No document names a system ref or admits
   a null one.
   **Blocks**: `cpt-cf-bss-products-dod-erasure-age`,
   `cpt-cf-bss-products-dod-retention-order`.
   **Owner**: `01-foundation`'s owner with this feature.

6. ~~**Where does a successful restore drill's state live?**~~ **Answered (P-D-134, 2026-09-04): an audit row per drill run**, P-D-21's own class; the last-verified watermark is the newest row per tenant — a query, not a table. *The item's text stood as:* `inst-rd-drill` puts "the last-verified
   watermark per tenant" on an operator surface; §4 says "retention/drill state is config + audit, no
   new record tables"; and a per-tenant watermark is neither config nor an append-only audit row.
   `DECISIONS.md` already records this as owed under P-D-21 and deliberately unapplied.
   **Blocks**: no DoD — **resolved by P-D-134** *(was: `cpt-cf-bss-products-dod-restore-drill`.)*
   **Owner**: this feature with P-D-21's owner.

7. ~~**What does the drill do on a digest-version mismatch, and what is the corruption alarm called?**~~ **Answered (P-D-133, 2026-09-04, the product owner): report, never skip, never re-render** — `unverifiable` rows raise `products_restore_drill_unverifiable`, a mismatch with code raises `products_restore_drill_corruption`; every row is scanned. *The item's text stood as:*
   The mismatch arm is named and not terminated — nothing says whether the row is skipped,
   re-rendered under its stored version, or reported — the alarm is unnamed where every other alarm
   in the set has a name, and the sample rule ("a sampled set") is unstated. *(Re-measured at
   `80eee534a`: `canonical::DIGEST_VERSION`'s doc constrains the answer — a bump "must arrive with
   the code that can still recompute the old rendering for those rows", or the drill "re-verifies
   every historical row against a rule it was never computed under and reports the whole table
   corrupt". What stays open is what the drill does when that code is absent.)* 01 pins `digest_version`
   starting at `1` as a code constant, so no second version yet exists to test the arm against.
   **Blocks**: no DoD — **resolved by P-D-133** *(was: `cpt-cf-bss-products-dod-restore-drill`.)*
   **Owner**: this feature with the NFR #5 workshop.

8. **Which surfaces may resolve an identity through the map, and under what grant?** *(P-D-117 (2026-09-03): **the engineering half is answered** — only the compliance export resolves through the map; `08` renders pseudonyms and `inst-im-render` is corrected. **What remains is item 11's half**: which principals hold `compliance × export`, Architecture's with Legal.)*
   `compliance × export` is "its own grant, never `audit × export`", and it is the only grant any
   document attaches to a map read — yet `inst-im-render` has approval queues and 08 projections
   resolving at render time. 08 states it renders "actor pseudonyms" and never mentions the map or a
   join, so the two slices disagree about what 08 does. 12 forbids *storing* an identity elsewhere
   and says nothing about resolving one.
   **Blocks**: `cpt-cf-bss-products-dod-identity-map`,
   `cpt-cf-bss-products-dod-compliance-export`.
   **Owner**: `05-governance`'s RBAC catalog owner with this feature and `08-read-models`.

9. ~~**Who owns NFR #5's cold re-resolution MUST, and how does the clause split with 06?**~~ **Answered (P-D-134, 2026-09-04): by object** — `10` owns identity cold re-resolution (the compliance export is it, P-D-117), `06` owns content cold re-read; NFR #5's p95 is the workshop's. *The item's text stood as:* Both slices
   claim the requirement as "shared", while 12 requires exactly one owner per clause. The word "cold"
   appears nowhere in the design set except this slice's Traces-to line: there is no instruction, no
   §4 shape and no §5 probe for cold resolution or its p95, and 06 — which owns the resolver — claims
   only the durability half.
   **Blocks**: no DoD — **resolved by P-D-134** *(was: `cpt-cf-bss-products-dod-restore-drill`.)*
   **Owner**: the design-set owner with `06-catalog-version`.

10. ~~**Who delivers the DR half of the durability mechanics?**~~ **Answered (P-D-133, 2026-09-04, the product owner): the platform's** — storage class, backups, RPO and RTO are deployment properties; the gear owns the restore drill as the probe. *The item's text stood as:* §1.1 promises "storage class, periodic
    checksum restore verification, DR posture" and §1.5 "checksum restore verification cadence, DR
    posture as config + probes", and the only body rule is the drill; storage class and RPO/RTO
    appear only as a constraint deferred to the NFR workshop. Either an instruction owes the DR
    config and probes, or Scope In owes a narrowing.
    **Blocks**: no DoD — **resolved by P-D-133** *(was: `cpt-cf-bss-products-dod-restore-drill`.)*
    **Owner**: the design owner, with the NFR workshop's output in hand.

11. ~~**Which actor holds `compliance × export`?**~~ **Answered (P-D-133, 2026-09-04, the product owner): a new `compliance × export` grant held by a platform compliance principal**, with justification and an audit row; Legal confirms the principal. *The item's text stood as:* The door returns real identities, the only
    compliance actor in §1.3 is described as reading "over pseudonymized trails", and 05's RBAC
    catalog has no such pair for this gear. Entangled with the erasure reach this feature's §2 closes at per-tenant
    (**P-D-50**).
    **Blocks**: no DoD — **resolved by P-D-133** *(was: `cpt-cf-bss-products-dod-compliance-export`.)*
    **Owner**: Architecture with Legal.

12. **Is `products_pii_allowlist` itself a PII store?** By construction it holds person-named *(P-D-117 (2026-09-03): **the posture half is answered** — the allow-list is a PII store by construction and takes the map's posture. **What remains is Legal's**: what an entry may contain.)*
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

14. ~~**What does "byte-identical in effect" mean for the age-triggered path?**~~ **Answered by P-D-117 (2026-09-03): the map state is what is byte-identical — tombstoned, payload destroyed, `principal_ref` standing — and the audit row differs by construction**, written under the system principal with the age rule's own name as the reason. §5 asserts the age path
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

16. ~~**`repo::AuditCommon::correlation_id` is permanently `NULL`, and this feature owns the audit *(P-D-118 (2026-09-03): **~~ **Decided by P-D-118 (recorded by P-D-134, 2026-09-04)**; the migration is the lead's, with P-D-129's columns. *The item's text stood as:*the shape is decided** — `correlation_id` is a W3C trace id, the column becomes `text`, background acts write NULL by design. **The migration edit is `01`'s and the lead's**; it does not block `dod-retention-clock`.)*
    class's clock and its deletes.** The field is `Option<Uuid>` and its own doc names the blocker:
    *"the value is 32 hex characters and this column is `uuid` on Postgres"*, with two candidate
    migration shapes written up and the decision, the migration behind it and the wiring recorded as
    **owed**. A retention sweep over the audit class deletes rows whose correlation column never
    carried a value, so the class this feature retains is less useful than its schema implies. The
    debt is `01-foundation`'s.
    **Blocks**: no DoD — **resolved by P-D-134** *(was: `cpt-cf-bss-products-dod-retention-clock` — not its correctness, its worth.)*
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

18. ~~**The map's read-by-principal path has no index of its own for the tombstoned case.**~~ **Answered by P-D-118 (2026-09-03): an index `(tenant_id, principal_ref, tombstoned_at)`**, landing with D's next migration — the allow-list table — on P-D-110/111's reasoning.
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

21. ~~**Does a principal have at most one live `actor_ref` per tenant, and is the erasure act therefore~~ **Answered by the crate, recorded by P-D-117 (2026-09-03)**: `uq_products_identity_ref_active` caps live rows at one and the shipped resolve is `.one(…)`. `inst-er-erase`'s plural named a population the index forbids and is corrected in `design/10`.
    single-row?** The partial unique index caps live rows at one and the shipped resolve uses
    `.one(...)`, but `design/10`'s `inst-er-erase` — the normative step — says *"resolves the
    operator identity to its `actor_ref`s and overwrites the map entries"*, plural. Correcting the
    plural here would put this document against a step it declares normative.
    **Blocks**: `cpt-cf-bss-products-dod-erasure-door`.
    **Owner**: `design/10`'s owner, on `inst-er-erase`'s wording, with the schema owner.

22. ~~**When a door-set fact differs between `design/10` §3.1 and `design/02`'s canonical enumeration,~~ **Answered by P-D-117 (2026-09-03): `design/02`'s canonical enumeration governs a door-set fact** — the slice that owns the hook owns the enumeration of what spends it. `design/10` §3.1 loses the bulk/promotion entry P-D-50 struck.
    which governs?** §1.4 pins precedence for column-level facts only. The bulk/promotion reason
    entry is a door-set fact: `design/02` records it **struck by P-D-50**, and `design/10` §3.1 still
    carries it because P-D-50's propagation reached `inst-er-export`, `inst-er-erase` and §6 — never
    `inst-im-map`. This document follows `design/02`; nothing states that it should.
    **Blocks**: `cpt-cf-bss-products-dod-pii-detector`.
    **Owner**: `design/10`'s owner with `design/02`'s.

23. ~~**What does an allow-list entry match on?**~~ **Answered by P-D-117 (2026-09-03): exact match on `value_normalized`** — C2's *curated allow-list for legitimate person-named products* is a list of names, not patterns. What makes the detector *uncertain* is the detector's own verdict, built against §6's matrix. Roster: item 31. No document names the table's central column, nor how
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

25. ~~**What is the GC's transaction boundary, and what does a crash mid-order leave?**~~ **Answered by P-D-118 (2026-09-03): one catalog version at a time, whole** — manifest, entries and the entity versions only it references in one transaction; a resumed pass re-judges from scratch. The stated
    order makes an intermediate state observable — a catalog-version row surviving with its entry
    rows deleted — in which the referential guard now *admits* deleting entity-version rows the
    surviving manifest still names, and a backup taken in that window carries a partial manifest the
    drill reports as a compliance incident. Nothing scopes the GC's transaction, says whether a pass
    is per-version or per-class, or says what a resumed pass does with a half-deleted version.
    **Blocks**: `cpt-cf-bss-products-dod-retention-order`,
    `cpt-cf-bss-products-dod-restore-drill`.
    **Owner**: this feature with `06-catalog-version`, which owns the predicate the order spends.

26. ~~**What `aggregate_id` do `ActorErased` and `PiiAllowlistChanged` partition on?**~~ **Answered by P-D-118 (2026-09-03): `ActorErased` on `principal_ref`, `PiiAllowlistChanged` on `entry_id`** — the thing each act serializes on.
    `infra::events::enqueue` requires one and **P-D-22** fixes
    `partition = hash(tenant_id, aggregate_id) mod N`. An erased actor and an allow-list entry are
    not aggregates in that sense. `features/catalog-version.md` §7 rows 27 and 47 reach the body core
    and the subject type and **neither reaches the partition key**.
    **Blocks**: `cpt-cf-bss-products-dod-retention-events`.
    **Owner**: the events/audit owner with the PRD §4.5 owner.

27. ~~**Is the audit-class retention window config or a DDL constant?**~~ **Answered by P-D-118 (2026-09-03): configuration, and the DDL guard admits any authorised `DELETE`.** The trigger stops *unauthorised* deletion; the *window* is the GC's own predicate from `retention_days_audit`. No `OLD.written_at < <cutoff>` arm is written. `design/10` §1.8 calls the
    retention durations config; `m20260829_000004` says *"A trigger cannot read configuration, so
    there is no predicate to write yet"* and plans the arm as a literal `OLD.written_at < <cutoff>`.
    If it is config the trigger cannot enforce it and the arm must admit any authorised `DELETE`; if
    it is a DDL constant an operator cannot set it per jurisdiction, which PRD §15 says Legal and
    Finance will supply.
    **Blocks**: `cpt-cf-bss-products-dod-retention-clock`.
    **Owner**: `01-foundation`'s owner, whose migration holds the guard, with Legal and Finance.

28. ~~**Three "configured" operands have no home.**~~ **Answered by P-D-118 (2026-09-03), and the premise was stale by nine fields.** PRD §15 already states the interim policy — *statutory max* — so five fields land: `retention_days_{financial,version,audit}` 3650, `pseudonymization_age_days` 730, `drill_cadence_hours` 24. Interim; Legal and Finance override by configuration. `ProductsConfig` ships exactly two fields —
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

30. ~~**Does this feature own the DR posture, and which DoD delivers it?**~~ **Answered (P-D-133, 2026-09-04, the product owner): no** — the DR posture is the platform's, the gear owns the drill; §1.1 and Scope In narrow, no thirteenth DoD. *The item's text stood as:* §1.2's divided-requirement
    table claims *"restore verification and the DR posture"*, §1.1 promises storage class and DR
    posture, and **no DoD names DR config or a DR probe**. Row 10 asks the question in the slice's
    terms and does not reach this document's own claim.
    **Blocks**: no DoD — it decides whether a thirteenth is owed or §1.1 and §1.2 narrow.
    **Owner**: the design owner with the NFR #5 workshop — row 10's owner.

31. ~~**Does `cpt-cf-bss-products-dod-pii-allowlist` owe a column roster?**~~ **Answered by P-D-117 (2026-09-03): yes** — `(tenant_id, entry_id, value_normalized, justification, signed_off_by, signed_off_at, state ∈ {active, revoked}, timestamps)`, `UNIQUE (tenant_id, value_normalized) WHERE state = 'active'`, revocation a state flip never a `DELETE` (P-D-47). The table does not exist yet; D's build. It is the only DoD here
    that creates a table and it names no column, while `cpt-cf-bss-products-dod-identity-map`
    restates ten column and guard names for a table that already ships. No document names a column of
    `products_pii_allowlist`. Either §1.4's restatement rule is narrowed to tables with a shipped
    schema, or `design/10` §4 is extended and this DoD restates it.
    **Blocks**: no DoD directly — it decides whether one is incomplete.
    **Owner**: the design owner with `05-governance`'s, which owns the `GovernedLiveOp` shape.

32. ~~**Is §6 owed one criterion per DoD?**~~ **Answered by P-D-118 (2026-09-03): yes.** A DoD whose unnamed obligations are *ticked by inspection* is a DoD nothing can fail. The five missing criteria are D's documentation work. Five DoDs have no criterion, among them the authz DoD whose
    own body argues that unnamed obligations are ticked by inspection. The general question is
    `features/catalog-version.md` §7 row 50's; this row records that it bites here too.
    **Blocks**: no DoD; it decides whether §6 is short.
    **Owner**: the design-set owner, as that row names.

### Owed to other documents, recorded and deliberately not edited

- **`features/foundation.md` §1.4** claims `artifacts.toml` excludes design slices from
  autodetection. It is false — `[systems.autodetect.artifacts.DESIGN_SLICE]` carries
  `pattern = "design/*.md"` and `traceability = "FULL"`. It is the shape donor for the four unwritten
  FEATUREs, so the cost compounds.
- **`repo::AuditCommon::correlation_id`**'s owed migration — see row 16. Owner: `01-foundation`. **Landed 2026-09-04** (P-D-118: `text` on both engines; the door writers fill it, background acts write `NULL`).

33. ~~**The detector's run heuristic, measured after it shipped (P-D-136, 2026-09-04): how much friction is the owner buying?**~~ **Answered (P-D-138, 2026-09-04, the owner): the `Uncertain` arm narrows to runs carrying a given-name dictionary word** — email and phone stay `Blocked`, uncertainty still blocks, the allow-list still lifts a signed-off run; strand D's build. *The item's text stood as:* `RegistryPiiDetector` returns `Uncertain` for **any run of two
    or more adjacent capitalized words** no active entry covers, and the hook refuses. Under the
    hook are localized attribute values, every operator reason, `displayLabel`, metadata values,
    the allow-list's own free text and the export justification — not names or codes. So with an
    empty allow-list an attribute value carrying *"Premium Cloud Backup"* is refused until Legal
    signs `premium cloud backup` off; a surname spelled *"McDonald"*, or a name in lowercase, is
    not a run at all and passes. This is C2's posture and item 1's recorded friction, shipped at
    its crudest so the allow-list loop is exercised before GA rather than after. **The question
    is the owner's**: keep the heuristic as the deliberate friction, or narrow it — a cap on the
    run's length, a field class it applies to, a dictionary of common words — before GA. Nothing
    here is a design gap; a narrower heuristic is a one-function change in `domain/retention.rs`.
    **Blocks**: no DoD — the detector's DoD is satisfied by the fail-closed hook whatever the
    heuristic. **Owner**: the product owner, with Legal's allow-list loop (item 1).

34. ~~**Does the catalog-version chain join the evidence class, or do its `DELETE` arms open?**~~ **Answered (P-D-137, 2026-09-04): the arms open, through a release stamp** — a catalog version is a financial record with a statutory window (PRD §330), not evidence; `retention_released_at` on the version row, admitted once by the whitelist and written only by the GC's release function under a writer-count guard, is the row-image fact the `DELETE` arm reads; entries and captures ride the parent. Both migrations in place, strand D's build. *The item's text stood as:*
    Measured 2026-09-04 while building `dod-retention-clock`: `products_catalog_version`
    (`m20260901_000010`) refuses every `DELETE` **outright and with no note**, and
    `products_catalog_version_entry` / `_capture` (`m20260901_000013`) refuse with an interim
    message naming *"slice 10's manifest retention"* — this feature — as their future admitter.
    **P-D-136** settled the five *evidence* migrations and does not reach these two. So the
    financial class is held at every candidate today, `dod-retention-order`'s *"capture and entry
    rows before their catalog-version row"* has no admitted delete to order, and P-D-118 item 25's
    *"one catalog version at a time, whole"* describes a transaction that always rolls back. Two
    readings, and the difference is a decade of storage: either a catalog version is evidence like
    an approval record and its guard is correct as it stands — in which case `m20260901_000013`'s
    interim message is stale and should say so — or the two arms open in place and the sweep
    begins collecting with **no code change here**, because it already offers the delete and
    reports the refusal. Neither of this feature's two clock DoDs waits on the answer: both are
    built against the guard **as it ships** and report what the engine decides, which is what
    makes them correct either way.
    **Blocks**: no DoD — it decides how much storage a decade costs, not whether the sweep is
    right.
    **Owner**: the design-set owner with `06-catalog-version`.

35. ~~**Does a retired head's last version expire with its window, or does the head keep it?**~~ **Answered (P-D-138, 2026-09-04, the owner): the head keeps it for as long as the head exists** — heads are append-only, so one frozen row per entity, ever; D's exclusion already covers every head state. *The item's text stood as:*
    P-D-137 (2026-09-04) rules that a version row **any head names as its current
    `published_version` is never a GC candidate** — the schema's only `DELETE` predicate is the
    manifest reference (P-D-40), and without the rule a live entity published once, long ago,
    would lose its only frozen content while its head named a row that no longer exists. That
    keeps one row per entity for as long as the head exists, and heads are physically append-only.
    What the decision did not settle is the **retired** head: PRD §15 says *"retention for retired
    entities/versions"*, which reads either as *the retired entity's versions expire on the
    window* (then the head's current pointer must be allowed to dangle, or be cleared by the GC)
    or as *retention applies to the entity's history, the head's own version included* (then the
    last version outlives the window by design). One row per retired entity is the cost of the
    second reading; a head that names nothing is the cost of the first.
    **Blocks**: no DoD — the clock and the order are built against the rule as decided.
    **Owner**: the product owner, with Legal (the same §15 sentence).
