# Feature: Catalog Version & Freeze

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-catalog-version-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-catalog-version`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Request → increment (the D-47 lanes)](#request--increment-the-d-47-lanes)
  - [Build the snapshot](#build-the-snapshot)
  - [Resolve a version (declared intent)](#resolve-a-version-declared-intent)
  - [The freeze protocol](#the-freeze-protocol)
  - [Grandfathering invariant](#grandfathering-invariant)
  - [`compositionPending` clearing](#compositionpending-clearing)
  - [Diff two versions (AC #20a)](#diff-two-versions-ac-20a)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Concurrency and ordering](#concurrency-and-ordering)
  - [Error taxonomy](#error-taxonomy)
  - [Observability: the posting-safe budget](#observability-the-posting-safe-budget)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The version row and its physical append-only guard](#the-version-row-and-its-physical-append-only-guard)
  - [The gapless allocator](#the-gapless-allocator)
  - [The manifest body: entries, captures and the P-D-40 index](#the-manifest-body-entries-captures-and-the-p-d-40-index)
  - [The P-D-40 referential predicate, by editing `m20260829_000007` in place](#the-p-d-40-referential-predicate-by-editing-m20260829_000007-in-place)
  - [The request queue](#the-request-queue)
  - [The freeze ledger and the participant set](#the-freeze-ledger-and-the-participant-set)
  - [The increment-request contract, in `bss-products-sdk`](#the-increment-request-contract-in-bss-products-sdk)
  - [The increment-request door](#the-increment-request-door)
  - [The coalescer and its per-tenant lease](#the-coalescer-and-its-per-tenant-lease)
  - [The snapshot builder and the first row collection through the canonical pin](#the-snapshot-builder-and-the-first-row-collection-through-the-canonical-pin)
  - [Stage-vs-commit re-validation, both arms](#stage-vs-commit-re-validation-both-arms)
  - [Version binding at freeze](#version-binding-at-freeze)
  - [The intentful resolver and byte-identity](#the-intentful-resolver-and-byte-identity)
  - [The ack door](#the-ack-door)
  - [The bounded timeout, and the export it owes `01-foundation`'s config](#the-bounded-timeout-and-the-export-it-owes-01-foundations-config)
  - [Force-completion, and the gate subject it cannot form](#force-completion-and-the-gate-subject-it-cannot-form)
  - [Participant-set governance](#participant-set-governance)
  - [Liveness records and the release door](#liveness-records-and-the-release-door)
  - [The grandfathering invariant, made auditable](#the-grandfathering-invariant-made-auditable)
  - [The `compositionPending` clearing lane](#the-compositionpending-clearing-lane)
  - [The diff door](#the-diff-door)
  - [The error taxonomy, wired into `DomainError`](#the-error-taxonomy-wired-into-domainerror)
  - [The authz surface, and the four rosters it reddens](#the-authz-surface-and-the-four-rosters-it-reddens)
  - [The four events, and the body shape none of the shipped types has](#the-four-events-and-the-body-shape-none-of-the-shipped-types-has)
  - [Delivery is a precondition of this feature, not a config note](#delivery-is-a-precondition-of-this-feature-not-a-config-note)
  - [The posting-safe meters](#the-posting-safe-meters)
  - [The audit trail for this feature's acts](#the-audit-trail-for-this-features-acts)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/06` §6](#carried-verbatim-from-design06-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

The point-in-time catalog every downstream gear pins to. This feature owns the `CatalogVersion`
machine — demand-driven mechanical increments over the ratified D-47 lanes, per-tenant
serialization, gapless monotonic ids — the full-snapshot builder with canonical serialization and
checksum, the resolution API with declared intent, the freeze protocol end to end, the
grandfathering invariant, the `compositionPending` clearing lane, the version diff, and the
posting-safe observability.

### 1.2 Purpose

Posted invoices and contracts must resolve the same bytes in five years that they froze today: the
`catalogVersion` segment of `pricingSnapshotRef` is this feature's product. Everything here is
**mechanical** (**P-D-02**) — governance happened at the entity publish, and an increment
snapshots already-governed content within a ratified SLO without ever waiting on a human.

**Requirements**: `cpt-cf-bss-products-fr-catalog-version-publish`,
`cpt-cf-bss-products-fr-catalog-publish-concurrency`,
`cpt-cf-bss-products-fr-catalog-version-diff`,
`cpt-cf-bss-products-fr-snapshot-reproducibility`,
`cpt-cf-bss-products-fr-freeze-atomicity`,
`cpt-cf-bss-products-fr-freeze-recovery`,
`cpt-cf-bss-products-fr-freeze-participant-governance`,
`cpt-cf-bss-products-fr-grandfathering-invariant`,
`cpt-cf-bss-products-fr-grandfathered-retention-coupling`,
`cpt-cf-bss-products-fr-bundle-adoption-guard`,
`cpt-cf-bss-products-fr-prepublish-lint`,
`cpt-cf-bss-products-fr-revision-vs-version`,
`cpt-cf-bss-products-nfr-posting-safe-budget`,
`cpt-cf-bss-products-nfr-snapshot-archival-dr`,
`cpt-cf-bss-products-nfr-publication-propagation`,
`cpt-cf-bss-products-nfr-scale-extensibility`

**Principles**: `cpt-cf-bss-products-principle-two-version-counters`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: `cpt-cf-bss-products-seq-catalog-version-freeze`

**Contracts terminated here**: `cpt-cf-bss-products-contract-increment-request`,
`cpt-cf-bss-products-contract-freeze-ack` (both halves — the acknowledgment and the
`catalog_version × release` door, **P-D-18**),
`cpt-cf-bss-products-contract-bundle-composition-signal`. Each is **declared by**
[`../design/06-catalog-version.md`](../design/06-catalog-version.md) §3 and cited here by id; a
`contract-` id stays at its slice.

**Divided requirements, and which half is this feature's.** Six of the sixteen above are not wholly
this feature's — four with a named counterpart half, one contested, and one with no deliverer at
all — and the division is the design set's rather than this document's:

| requirement | this feature's half | the other half |
|---|---|---|
| `fr-grandfathered-retention-coupling` | the per-version freeze-registration records — the liveness **source** | the retention gate that reads them (`10-retention-erasure`, `inst-rt-gc`) |
| `fr-revision-vs-version` | the version-binding-at-freeze clause | the two counters and the version history (`01-foundation`) |
| `fr-bundle-adoption-guard` | the registry half — the `compositionPending` clearing lane | the pricing-side composition signal, unregistered (PRD §15) |
| `nfr-snapshot-archival-dr` | the archival and snapshot operand — **see §7** | restore verification and the DR posture (`10-retention-erasure`) |
| `nfr-publication-propagation` | **contested — see §7.** The slice claims the freeze-machine half of the < 3 s budget and `01-foundation` claims the outbox half, and both slices record the split as unsettled in their own open items | — |
| `fr-prepublish-lint` | claimed by the slice and scoped In by its §1.5 | **nothing delivers it — see §7.** No instruction, store, grant, code or probe in the design set produces the report `09-bulk-promotion` consumes |

**Out of scope**: entity publish and its governance (`01-foundation`, `05-governance`); what a
participant does with the fan-out, which belongs to its own gear; retention and GC **execution**
(`10-retention-erasure` — this feature supplies the liveness records, **P-D-40**'s referential
`DELETE` predicate and the index that predicate needs, and nothing else); `pricingSnapshotRef`
composition, which is rating's; the pricing-side pending-ref table, which pricing owns.

**Not applicable**: no state machine is declared here — see §4.

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Operator-initiated catalog publish; freeze monitoring, ack re-trigger, force-completion ceremony, participant-set governance |
| `cpt-cf-bss-products-actor-plan-price` | Requests addressability through the increment contract (D-47 pending ref); owes a `freezeComplete` ack; sends the composition signal |
| `cpt-cf-bss-products-actor-contracts` | Freeze participant. A silent counterpart today (PRD §15) and **not in the v1 registered set** (**P-D-48**) |
| `cpt-cf-bss-products-actor-billing` | Freeze participant, on the same footing as Contracts |
| `cpt-cf-bss-products-actor-subscriptions` | Grandfathering beneficiary: a frozen snapshot it holds never moves |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md) — §6.5 (`fr-revision-vs-version`, whose version-binding-at-freeze
  clause §1.2 divides), §6.6 (all **eight** FRs it defines, `fr-catalog-version-diff`
  included), §6.13 (`fr-catalog-publish-concurrency`, `fr-grandfathered-retention-coupling`
  liveness-source half, `fr-prepublish-lint`), §7 (the posting-safe, propagation, archival and
  scale NFRs); AC #19–#25, #20a, #40, #44, #45
- **Design**: [DESIGN.md](../DESIGN.md); the granular module boundary is
  [`../design/06-catalog-version.md`](../design/06-catalog-version.md) (434 lines)
- **Decisions**: [DECISIONS.md](../DECISIONS.md) — **P-D-02** (mechanical increments),
  **P-D-06** (metadata capture), **P-D-09** (stage-vs-commit fail-closed per lane), **P-D-13**
  (`quorumReduced` on force-completion), **P-D-14** (the composition clear is not exempt from the
  gate), **P-D-18** (the release door), **P-D-19** (posted resolution of a force-completed
  version), **P-D-46** (`closed_at` struck), **P-D-47** (the per-version auto-fallback opt-in
  withdrawn from v1), **P-D-48** (the v1 participant set narrowed to pricing), **P-D-49**
  (`participant_set_snapshot` as the liveness operand), **P-D-50** (`satisfied_by_version_id` as
  a column). Cross-gear: pricing **D-47** (the lanes and the SLO — the joint contract)
- **Dependencies**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-taxonomy-attributes`,
  `cpt-cf-bss-products-feature-sku-classification`,
  `cpt-cf-bss-products-feature-lifecycle`, `cpt-cf-bss-products-feature-governance`

**The declaration site, and what this document may define.**

`design/06`'s seven `flow-` and two `algo-` declarations **moved here**; each of its sections now
carries a pointer at this file and keeps its own instruction steps, which stay normative. One
definition site per id.

This document defines only `flow`, `algo`, `dod` and `featstatus` ids. **It declares no `inst-`
id**: §4 mints none because this feature has no state machine (see §4), so every `inst-` reference
below resolves into a design slice.

`design/06` §3.2 carried only `cpt-cf-bss-products-contract-cv-errors`, and a `contract-` id stays
at its slice. So **`cpt-cf-bss-products-algo-catalog-version-errors` is minted here** and §3.2
points at it — the fifth time a `contract-`-only §3 section has needed this.

**This document does not copy the slice's instruction steps.** §2 and §3 carry the actor, the
scenarios and the boundary; the steps stay at their single source. §5 does restate storage shapes,
because a DoD must name the columns it obliges — **`design/06` §4 governs on any column-level
fact**, and its optionality markers (`operation_key?`, `not_frozen(forced_at, ceremony_ref)`,
`released_at`, `satisfied_by_version_id`) are kept here deliberately.

**The five foreign instruction ids this feature reaches, and which two are unwritten.**

| id | slice | FEATURE written? |
|---|---|---|
| `inst-fd-publish-txn` | `01-foundation` | yes — `features/foundation.md` |
| `inst-fd-publish-emit` | `01-foundation` | yes — `features/foundation.md` |
| `inst-pr-snapshot` | `07-reference-signal` | **no** |
| `inst-rt-gc` | `10-retention-erasure` | **no** |
| `inst-fd-idem-retention` | `01-foundation` | yes — `features/foundation.md` |

Five is the lowest foreign-seam count of any capability feature written so far; `05-governance`'s
was twelve. The fifth is the only one this feature **writes into** rather than reads from:
`cpt-cf-bss-products-dod-freeze-timeout` obliges the `max_freeze_timeout` export that
`inst-fd-idem-retention`'s floor is missing, and `features/foundation.md` already carries the
counterpart clause. Two further foreign ids — `inst-fd-rule-registry` and `inst-lc-undeprecate` —
appear only inside §7 re-measurement notes and are **not** seams this feature takes. The two unwritten ones are the reference-producer set's symmetric snapshot ride and the
retention GC that reads this feature's liveness records — neither is a write path this feature
takes.

**Positive findings against the shipped crate.**

Three claims the design set makes about code were byte-verified at `41d1baa5e` and **hold**. They
are recorded because a later reader will otherwise re-measure them. *Measurements in this document
dated `41d1baa5e` still stand at `c081872ab`: no `.rs` file changed between the two commits, both
being documentation-only.*

- **`inst-cc-clear`'s mechanism is exactly what ships.** `composition_pending` is a
  `products_sku` column only, and `infra/storage/repo.rs` carries it as a **parameter** of the
  publish door's head-row `UPDATE` twin — the statement whose own doc says `composition_pending`
  *"rides this statement and can ride no other"*. A save writes no version row and cannot move the
  flag. The design's claim that 01 §4.2 admits the change only in that statement (**P-D-32**) is
  the code's own arrangement.
- **`inst-cvc-order` is true — on a mechanism the slice does not name.** Read against
  `infra::events::partition_for`, whose key is the `(tenant_id, aggregate_id)` **pair**, per-tenant
  version ordering would be false: versions would spread across partitions and no cross-partition
  order exists. It holds because under **P-D-47** the consumer-visible order comes from the
  broker's own selection — `MurmurHash3-32` over **`tenant_id` alone**, recomputed at ingest —
  with `partition_for` demoted to a *pipeline* invariant that the same module doc calls superseded
  *"for the guarantee a consumer actually reads"*. **Cite the broker's selection, not
  `partition_for`.**
- **The increment port already ships on the counterpart side, and it pre-agreed to become an
  adapter.** `bss_pricing_sdk::CatalogVersionRegistryV1` exists, is wired with a fail-closed
  default, and `request_version` is reached from **twelve production call sites in ten modules** —
  the count `infra::registry_deadline`'s own doc states — every one of them through
  `infra::registry_deadline::request_version_now`; plus a single `committed_version` caller in
  `infra::jobs::readmodel_warm`. Its module doc: *"The contract lives here, in the catalog's
  own SDK, because the registry gear has no code in this repository yet. That is a temporary
  asymmetry, not a claim of ownership: when the registry publishes its own SDK this trait becomes
  an adapter over it."* So `design/06` §1.7's *"The contract is the `products-sdk`
  increment-request client, not a transport"* is right, and the new dependency edge runs
  `bss-pricing` → `bss-products-sdk`. The four
  measured mismatches between that port's shape and this feature's contract are in §7; they are
  seam questions, not defects in either side.

**Pricing's dev-minted version space collides with this feature's counter.**

Pricing
ships `LocalDevCatalogVersionRegistryV1`, selectable by `mode = "local_dev_invented_versions"`,
which mints versions from `Utc::now().timestamp_millis()` — of the order 10¹² — behind a boot
`warn!` reading *"Never run this beside the Product & SKU registry."* This feature's counter is
expected to start low — **no document pins its initial value**, which is part of what §7 row 23
asks; `CatalogVersion` is `Ord`; and pricing's pin-eligibility frontier is
**prefix-closed**. So on any deployment that ran in that mode, every version this registry issues
sorts **earlier** than every locally minted one, and the frontier cannot advance past the
contamination. The shipped code names the sweep — `LIKE 'dev-local-%'` over
`pricing_plan_revision.pending_version_ref` — and assigns it to nobody. Registered in §7; not
answered here.

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-freeze-monitoring`

Each flow below is **declared here and stepped in
[`../design/06-catalog-version.md`](../design/06-catalog-version.md) §2**, whose steps are the
normative ones. What this section carries is the triggering actor, the scenarios and the boundary.

### Request → increment (the D-47 lanes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-increment`

**Actor**: `cpt-cf-bss-products-actor-plan-price` (and
`cpt-cf-bss-products-actor-catalog-admin` for the operator publish act)

**Success Scenarios**:
- An interactive request coalesces with any other pending interactive request within ≤ 5 s of the
  earliest, and one version commits carrying both.
- A keyed **bulk** batch stays open until the 5-minute hard max from its earliest request and
  lands as **one** version, whatever interactive versions publish in between. There is no
  early-close signal — **P-D-46** struck `closed_at`.
- A retry on the same `(source, request_key)` is answered by the first request's outcome and
  enqueues nothing.
- `CatalogVersionPublished` carries the changed-entity list against the immediately previous
  version and the `satisfiedRequests` set — the `(source, request_key)` pairs this version
  committed.

**Error Scenarios**:
- A request whose `source` is not a registered requester is refused at the door.
- A pending request past the lane deadline raises `catalog_version_overdue`; it is an **alarm, not
  a refusal**, and the request stays queued.
- **No approval is ever consulted** (C2). An increment cannot fail for want of one.

**Boundary**: the door and the coalescer are this feature's; the **entity publish** that produced
the content is `01-foundation`'s and never enqueues an increment of its own — a retirement's
`effectiveAt` flip likewise does not, the next demand-driven version reflecting it. The pricing-side
pending-ref table is pricing's.

### Build the snapshot

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-snapshot`

**Actor**: the increment worker (system), on behalf of
`cpt-cf-bss-products-actor-plan-price` or `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- Every published or deprecated entity contributes a **reference** into its immutable
  `products_entity_version` row; every live capture — **all seven kinds**: category tree and
  display values, attribute definitions, category values, per-entity metadata maps (**P-D-06**), the
  recognized sets, the freeze-participant set snapshot (AC #23) and the reference-producer set
  snapshot (`inst-pr-snapshot`'s symmetric ride) — is a **stored canonical copy**, never a
  reference.
- The manifest is canonically serialized and checksummed, and a re-resolution renders from the
  stored manifest rather than re-collecting.
- The resolve or finalize response carries `(boundVersion, resolvedVersion, diffRef)` when a
  bound-not-yet-frozen reference re-resolves to a newer version — the diff is surfaced **to** the
  module rather than left for it to know to pull.

**Error Scenarios**:
- An entity whose published version **or lifecycle state** moved between collect and commit fails
  the run closed: `STAGED_ENTITY_CHANGED` **naming the entity** on the operator lane, while a
  mechanical run re-coalesces and retries fresh with the request never lost. The lane split is
  normative under **P-D-09**.

**Boundary**: the snapshot references sibling-gear content and never copies it (C4). The
canonical rendering rule itself is `01-foundation`'s single pin — this feature brings the **first
row collection** through it, and the nested-roster and collection-sort arms that implies land in
`domain::canonical`, which is where that module's own doc already says they are owed.

### Resolve a version (declared intent)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-resolve`

**Actor**: `cpt-cf-bss-products-actor-plan-price`,
`cpt-cf-bss-products-actor-contracts`, `cpt-cf-bss-products-actor-billing`,
`cpt-cf-bss-products-actor-subscriptions`

**Success Scenarios**:
- `intent = browse` serves any published version at once.
- `intent = posted` serves a version whose ledger reads complete.
- Re-resolution is byte-identical forever, and the checksum is returned and verifiable.

**Error Scenarios**:
- An absent `intent` is `INTENT_REQUIRED` — a consumer-side obligation the seam suite asserts.
- `posted` against an incomplete freeze is `FREEZE_INCOMPLETE`.
- `posted` against `complete(forced)` is `VERSION_FORCED_INCOMPLETE`, **naming each
  `not_frozen(forced)` participant**, and stays refused until every forced participant has since
  frozen or released **through its own door**. The predicate reads `state`, so a row left
  `not_frozen(forced)` with `released_at` stamped by the ceremony does **not** satisfy it —
  otherwise force-completion would discharge its own refusal inside the transaction that raises it.
- An unknown `catalogVersionId` is `CATALOG_VERSION_UNKNOWN`, raised by **one** door: the shared
  version-lookup component behind both resolve and diff.
- **There is no second disjunct in v1** (**P-D-47**): the per-version auto-fallback opt-in stays an
  off-by-default later enhancement, so no column, door or ceremony carries it. A version whose
  forced participant never returns is **superseded, not rescued**, and stays refused; `browse` is
  unaffected in every case.

**Boundary**: refusing a `not_frozen` participant's **content** is a consumer seam obligation
booked in `12-consumer-contracts`. Refusing the **version** is this feature's door outright.
`08-read-models` explicitly places this surface out of its own scope.

### The freeze protocol

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-freeze`

**Actor**: `cpt-cf-bss-products-actor-plan-price` (ack, release),
`cpt-cf-bss-products-actor-catalog-admin` (monitoring, re-trigger, force-completion)

**Success Scenarios**:
- On `CatalogVersionPublished`, every participant in the **version's snapshotted set** owes an ack;
  `freezeComplete` is all-acked (AC #21). Acks are idempotent per `(version, participant)`.
- The fan-out re-trigger is idempotent — same event, same version, at-least-once safe.
- Force-completion is a two-person ceremony, `N`-governed, recording `quorumReduced` on the record
  **and on `FreezeForceCompleted`** below the default of 2 (**P-D-13** — no fixed floor, since one
  would leave a solo tenant's timed-out version permanently un-resolvable).
- Participant-set membership is a governed live op; each change emits
  `FreezeParticipantSetChanged`, because a participant must learn it was added. Each version
  resolves `freezeComplete` against **its own snapshotted set** forever — removal after publish
  never retro-flips a historical version (AC #23).
- A release through the participant's own door records that it holds no more live references to
  that version.

**Error Scenarios**:
- An ack from a principal outside that version's **snapshotted** set is `PARTICIPANT_UNKNOWN`,
  refused and audited. This is a **membership** check, not authentication.
- The bounded timeout fails **closed**: past it the version stays non-posting-safe and
  `freeze_overdue` names the silent participants.
- Ceremony refusals ride `05-governance`'s own gate codes. There is **no**
  `FORCE_COMPLETE_QUORUM` — the design set names that token only in order to say it does not
  exist.

**Boundary**: the ack door accepts a participant's ack only under that participant's own service
identity. What a participant does to freeze its content is its own gear's. The retention gate that
reads these records is `10-retention-erasure`'s (`inst-rt-gc`); this feature supplies the records
and the release door only.

### Grandfathering invariant

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-grandfathering`

**Actor**: `cpt-cf-bss-products-actor-subscriptions`

**Success Scenarios**:
- A frozen snapshot referenced by a grandfathered consumer is **never mutated** — held by
  construction, since entity versions are append-only under 01's history rows, manifests are
  append-only under this feature's storage, and retirement and deprecation touch head rows only.

**Error Scenarios**: none. There is no door to refuse; the flow exists so that the delegation is
**auditable from the registry side**.

**Boundary**: eligibility policy is plan-price's and subscriptions-lifecycle's. **The immutability
is this feature's**, and that division is the whole content of the flow.

### `compositionPending` clearing

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-composition-clear`

**Actor**: `cpt-cf-bss-products-actor-plan-price` (the inbound signal is the authorizing
principal)

**Success Scenarios**:
- On a **clean head**, the inbound signal clears `composition_pending` as a system save plus a
  re-publish of the head at version N+1, carrying no uncomposed-bundle override so that
  **P-D-30**'s predicate is false and the flag is not re-raised. The act is **not exempt from the
  gate**: it runs as a `system_signal` approval subject auto-satisfied by the signal itself as the
  authorizing principal, recorded on the `ApprovalRecord` with the signal reference. That
  satisfaction is **independent of the tenant's configured `N`** (**P-D-11**) — a `system_signal`
  subject neither consumes the human quorum nor is exempt from the gate, because the governance for
  this act already happened pricing-side.
- The flag stays system-owned and never operator-mutable; `SkuCompositionCleared` is emitted —
  **this gear's outbound event, distinct from the inbound `BundleCompositionCompleted` that drove
  it**. Prior frozen versions keep the flag as it was (C4).

**Error Scenarios**:
- The clear raises **no error code by design**. Its caller is a signal, not a request, so a
  blocked clear is an **alert plus a retained flag** rather than a refusal a producer would have to
  interpret.
- On a **dirty head** — any unpublished local edit or open approval, `taxCategory` and `PlanTier`
  among them — the clear is **deferred, never refused** (**P-D-14** as confirmed by **P-D-48**).
  The signal is durable and idempotent, `composition_pending` stays `true`, a
  `composition_clear_held` alert names the entity and the blocking edit or approval, and the clear
  re-evaluates when the head next goes clean. This is the third instance of one guard:
  `CORRECTION_DIRTY_HEAD` / `CORRECTION_APPROVAL_OPEN` (`07`) and `PROMOTION_DIRTY_HEAD` (`09`) are
  the other two.

**Boundary**: the publish this clear performs is `01-foundation`'s door
(`inst-fd-publish-txn`, `inst-fd-publish-emit`); this feature owns the trigger, the deferral rule
and the flag's system ownership. The pricing-side signal is unregistered today (PRD §15).

### Diff two versions (AC #20a)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-diff`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`,
`cpt-cf-bss-products-actor-plan-price`

**Success Scenarios**:
- The diff covers **every snapshot member**: entities added and removed, per-entity
  published-version deltas rendering 01's history diff, **and the capture half** — all seven kinds:
  category tree and display values, attribute definitions, category values, recognized sets,
  per-entity metadata maps, and the participant and producer sets. A metadata-only or
  live-entity-only change between two versions **must** appear; the manifest's own membership is the
  diff's universe.
- It is computed read-only from the two stored manifests, is byte-stable for a given pair, and has
  **no retention effect**.

**Error Scenarios**:
- Either unknown `catalogVersionId` is `CATALOG_VERSION_UNKNOWN`, from the same single door as
  resolve.

**Boundary**: read-only. The diff neither extends a version's liveness nor creates a
freeze-registration row.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and stepped in
[`../design/06-catalog-version.md`](../design/06-catalog-version.md) §3**, whose steps are the
normative ones.

### Concurrency and ordering

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-cv-concurrency`

**Input**: the pending `products_catalog_version_request` rows of one tenant, and that tenant's
`products_catalog_version_counter` row.

**Output**: a committed `products_catalog_version` row at a gapless, strictly monotonic id, and
one `CatalogVersionPublished` whose changed-entity list is computed against the immediately
previous version **inside the same transaction**.

**Boundary and the mechanism to name.** One increment worker per tenant. `design/06` writes this
as *"advisory lock / queue partition"*; the house mechanism is neither — `gears/bss/libs/coord`
ships a DB-backed distributed lease (`LeaseManager`, `LeaseGuard::spawn_renewal`, and
`LeaseGuard::with_ack_in_tx`, a write fence), on which both `bss-pricing` and `bss-ledger` already
depend. **A per-tenant key is precedented, not new**: `bss-pricing` keys its three job leases per
gear and per pass, while **`bss-ledger` already keys two per tenant** —
`recognition-run:{tenant}:{period_id}` and `period-close:{tenant_id}:{legal_entity_id}:{period_id}`
— and `coord`'s own README offers one per `(tenant, period)` as a typical fit. What remains a
decision is the **cardinality cost**, recorded in §7.

Entity publishes are **not** blocked by a running increment — they land on heads, and the
re-validation step decides whether the run must retry. Fan-out ordering per tenant is the version
order by construction, **through the broker's own partition selection over `tenant_id`** under
**P-D-47**, not through `events::partition_for`, whose key is the `(tenant, aggregate)` pair.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-catalog-version-errors`

**Input**: a refusal raised by any door of this feature.

**Output**: a `DomainError` variant carrying its wire code, and an RFC 9457 problem response at
the mapped status.

**The roster is six**, and it is normative at
[`../design/06-catalog-version.md`](../design/06-catalog-version.md) §3.2, which keeps
`cpt-cf-bss-products-contract-cv-errors`:

| code | status | raised by |
|---|---|---|
| `INTENT_REQUIRED` | **422 architectural → 400 on the wire** | `inst-rv-intent` |
| `FREEZE_INCOMPLETE` | 409 | `inst-rv-intent` |
| `VERSION_FORCED_INCOMPLETE` | 409 | `inst-rv-intent` |
| `STAGED_ENTITY_CHANGED` | 409 | `inst-sn-revalidate`, operator lane only |
| `CATALOG_VERSION_UNKNOWN` | 404 | the shared version-lookup door, resolve **and** diff |
| `PARTICIPANT_UNKNOWN` | 403 | `inst-fz-ack` |

**Four tokens in the slice are not codes of this feature.** `STALE_VERSION`,
`CORRECTION_DIRTY_HEAD`, `CORRECTION_APPROVAL_OPEN` and `PROMOTION_DIRTY_HEAD` belong to `01`,
`07` and `09` and are cited. **A fifth, `FORCE_COMPLETE_QUORUM`, is named in the slice in order to
state that it does not exist** — force-completion refusals ride `05-governance`'s gate codes. A
reader counting screaming-case tokens in that file arrives at eleven; the roster is six.

**The 422 here is architectural, not wire — and it is a choice, not an impossibility.** The reason
is **not** that *no path can produce* a 422: `design/01-foundation.md` §3.3 records **that** premise
as measured false and retired by an owner's call of 2026-08-27, since a transport override can move
a single occurrence's wire status inside its status class and 422 is in `FailedPrecondition`'s. The
platform having **no 422 category** is a separate fact that §3.3 still quotes as governing, and it is
not what was retired. What makes the rule true is the property it protects — *"this gear declares
no transport override anywhere, and neither does pricing — so every registry code has exactly one
wire shape"*. So `INTENT_REQUIRED` reaches the wire as a **400** carrying its code, no endpoint may
declare a 422 for an error carrying a registry code in OpenAPI, and a bare 400 stays reserved for a
malformed request. `error_mapping_tests::the_products_owned_422_codes_stay_wire_400_by_design`
guards the **three** codes shipped today (`VALIDATION`, `SCOPE_NOT_CONTAINED`, `INCOMPLETE_ENTITY`)
as a **hard-coded array**, not a sweep, so it cannot see a fourth — which is why
`cpt-cf-bss-products-dod-cv-error-taxonomy` obliges `INTENT_REQUIRED`'s row in it. Reaching for
`Http::status_code` is what that test exists to catch.

**The composition clear raises nothing** (`inst-cc-clear`) — see §2.

### Observability: the posting-safe budget

- [ ] `p2` - **ID**: `cpt-cf-bss-products-algo-posting-safe`

**Input**: the request queue's timestamps, the freeze ledger's ack timestamps, and the outbox's
own commit-to-acceptance meter.

**Output**: three meters — `requested → published` (C1), `commit → event durably accepted`, and
`event → ack` per participant — plus the gauges and alarms below. The posting-safe composite is
declared **derivable from these three without a fourth clock**.

**Gauges and alarms**: pending-request age per lane; unacked participants per version;
`freeze_overdue`; `catalog_version_overdue` — the registry-side mirror of pricing's
`commit_overdue`.

**The middle meter is declared by no slice.** §3.3 attributes `commit → durable-acceptance` to
`01-foundation`; that slice declares no observability surface and records its own NFR #3 probe as
owed, and `08-read-models` also names the meter as 01's. So a composite declared derivable from
three meters rests on one that is nobody's. Carried to §7 rather than assigned here.

## 4. States (CDSL)

**No state machine is declared by this feature, and that is a measurement rather than an omission.**

`design/06` declares no `state-` id. The two value rosters this feature stores are storage
columns, normative at that slice's §4:

- `products_catalog_version.freeze_state ∈ {open, complete, complete(forced)}` — annotated in the
  slice as a **derived cache of the ledger**, so it is a projection rather than an independently
  driven state.
- `products_freeze_ack.state ∈ {pending, acked, released, not_frozen(forced)}` — **four values,
  one column**, with **six admitted transitions and no others** (**P-D-60**). The retention-release
  fact rides its **own column**, `released_at`, precisely so
  that it cannot be read as an ack or as a release through the participant's own door.

Because §4 declares nothing, **this feature mints no `inst-` id at all** — unlike
`features/lifecycle.md`, whose §4 machine required one step id per transition row. Every `inst-`
reference in this document resolves into a design slice.

**And the transition table for the second roster is genuinely unstated** — whether `pending` may
go straight to `released`, whether force-completion may overwrite a row already `acked` or
`released`, and whether a forced participant's later ack clears the `released_at` the ceremony
stamped. Each answer moves both `freezeComplete` and `10-retention-erasure`'s collection gate,
which reads the `(state, released_at)` **pair**. That is why no machine is invented here: a
machine authored in this document would be authoring the answer. §7 carries the question.

## 5. Definitions of Done

Every DoD below names types, functions, tables and tests **that exist at `c081872ab`** wherever
one exists, rather than inventing a shape. Where the shipped seam cannot host what this feature
needs, the DoD says so and §7 carries the question.

### The version row and its physical append-only guard

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-catalog-version-table`

The system **MUST** create `products_catalog_version`, keyed `(tenant_id, catalog_version_id)`
with the id monotonic per tenant, carrying `checksum`, `staged_at`, `published_at`,
`participant_set_snapshot`, `freeze_state ∈ {open, complete, complete(forced)}` and the manifest
header, on **both** engines.

The table **MUST** be append-only and **physically guarded** — but on the **whitelist**
discipline, **not** the unconditional refusal `m20260829_000007_create_products_entity_version.rs`
ships. That migration refuses every `UPDATE` outright, and says why: *"there is no admitted
`UPDATE` for one to describe — unlike the head tables one migration over, where the whitelist
exists precisely because some updates are legitimate."* **This table has an admitted update**:
`cpt-cf-bss-products-dod-force-completion` obliges `freeze_state` to move to `complete(forced)`, and
whichever act §7 row 38 assigns must move it to `complete`. Mirroring the unconditional guard would
make the whole `complete(forced)` path uncommittable.

So the model is `m20260829_000002_create_products_product.rs`'s head-row guard
(`cpt-cf-bss-products-dod-append-only-guard`): on Postgres one `PL/pgSQL` function branching on
`TG_OP` behind a single trigger firing `BEFORE DELETE OR UPDATE`, comparing `NEW` against `OLD`
with `IS DISTINCT FROM`; on SQLite, which has no procedural language and whose `RAISE(ABORT, …)`
takes a literal message, the same whitelist split across **one no-delete trigger and one
`WHEN`-guarded trigger per column class**, using `IS`/`IS NOT`. **`freeze_state` MUST be the only
column the `UPDATE` arm admits**; every other column of this table — `checksum`, `staged_at`,
`published_at`, `participant_set_snapshot` and the manifest header — **MUST** be refused, since the
byte-identity flagship rests on them. `DELETE` **MUST** be refused outright.

Both refusal messages — the delete arm's and the update arm's — **MUST** be asserted **apart**,
because a body that lost its `UPDATE` branch would still refuse an update with the delete message:
same outcome, different guard, and only the text tells them apart.

**`freeze_state` MUST NOT be the authority.** The column is a derived cache; the ledger is the
operand every predicate reads. Admitting its write is a storage permission, not a promotion.

**Implements**: `cpt-cf-bss-products-flow-increment`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_catalog_version`
- Entities: `CatalogVersion`

### The gapless allocator

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-version-counter`

The system **MUST** create `products_catalog_version_counter`, keyed `(tenant_id)`, holding the
next id, and **MUST** allocate from it **inside the same transaction as the version insert**, so
that ids are gapless by construction rather than by a reconciliation pass.

A refused run **MUST NOT** consume an id. This is what forbids an insert at stage time: an insert
before publication would burn an id on every `STAGED_ENTITY_CHANGED` refusal, against the gapless
guarantee C1 and `inst-cvc-serial` both assert. **The `staged_at` column therefore has no admitted
writer in the design set as it stands** — §7.

**Implements**: `cpt-cf-bss-products-flow-increment`

**Touches**:
- DB Table: `products_catalog_version_counter`
- Entities: `CatalogVersion`

### The manifest body: entries, captures and the P-D-40 index

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-version-entry-table`

The system **MUST** create `products_catalog_version_entry`, keyed
`(tenant_id, catalog_version_id, entity_kind, entity_id)` → `published_version`, holding
**references** into immutable `products_entity_version` rows; and **MUST** create
**`products_catalog_version_capture`** as a table of its own (**P-D-60**), keyed
`(tenant_id, catalog_version_id, capture_kind)` → the **stored canonical copy** of the
category tree and display values, attribute definitions, category values, metadata maps,
recognized sets, freeze-participant set and reference-producer set as of the snapshot. Live content
is **copied, never referenced**. Both are append-only and the checksum covers both halves, being
computed over content rather than over a table.

The system **MUST** additionally index `(tenant_id, entity_kind, entity_id, published_version)` —
**not** for a read of this feature's own, but because the primary key leads with
`catalog_version_id` while **P-D-40**'s retention `DELETE` predicate on `products_entity_version`
must look a row up by its entity coordinates. Without this index the predicate is a scan of every
version of every tenant.

**The capture store is its own table** (**P-D-60**), and that is what keeps this index honest: one
PK cannot express both keys, a shared table would make every column of both halves nullable —
admitting a row that is neither a valid entry nor a valid capture — and **P-D-40 needs no
re-aiming**, its predicate being written over `products_catalog_version_entry`, whose every row now
references an entity version. Capture rows reference nothing, so they never participated in that
predicate.

**Implements**: `cpt-cf-bss-products-flow-snapshot`

**Touches**:
- DB Table: `products_catalog_version_entry`, `products_catalog_version_capture`
- Entities: `CatalogVersionEntry`, `VersionManifest`

### The P-D-40 referential predicate, by editing `m20260829_000007` in place

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-referential-delete-predicate`

The system **MUST** replace `products_entity_version`'s **unconditional** `DELETE` refusal with
**P-D-40**'s referential predicate: a frozen row may be deleted only when **no
`products_catalog_version_entry` references it** — a table whose **every row now references an entity
version**, the capture rows having moved to `products_catalog_version_capture` (**P-D-60**), so the
predicate and its index are exactly right as written and **P-D-40 needs no re-aiming**. The change **MUST** be made **by editing
`m20260829_000007_create_products_entity_version.rs` in place** — this migration chain edits
migrations in place and does not chase them with tightening ones — and **MUST** land on both
engines: the Postgres `PL/pgSQL` `TG_OP` arm and the SQLite `trg_products_entity_version_no_delete`
trigger.

This DoD is the reason this feature was built sixth. The shipped migration names it in five places,
including the refusal message itself on both engines — a plain SQL string literal reading
`products_entity_version is frozen: DELETE is not permitted until the referential predicate lands with products_catalog_version_entry`
— and its own module doc:
*"the referential arm is **owed to slice 06**"*.

Two shipped **green** tests assert the interim rule and **MUST** be amended, not deleted:

| test | tier |
|---|---|
| `migrations_tests::entity_version_guard_tests::a_delete_of_a_frozen_row_is_refused` | default (SQLite) |
| `postgres_frozen_guards::a_frozen_version_row_admits_neither_update_nor_delete` | pg (Docker-gated) |

And **one owed probe lands with this predicate**: 01 §5 requires that deleting a
`products_entity_version` row a `products_catalog_version_entry` **still references** be refused
**by the guard, not merely skipped by the GC** — *"a probe that passes when the GC is bypassed
entirely"*. Its premise did not exist before this feature; it does now, so the probe is owed here
and **MUST** be written.

**The direction of this change is the risky one, and that is why the probe is not optional.** The
migration's own doc argues that the interim unconditional refusal is *strictly stronger* than the
predicate it stands in for, so shipping it early could not admit a delete the final predicate would
refuse. This DoD **loosens** the guard, so its correctness rests entirely on the probe.

**This DoD does not tick `cpt-cf-bss-products-dod-version-history-table`.** That DoD has a second
open clause — the canonical serialization pinned by a golden vector *asserted byte-identical across
engines* — which is `01-foundation`'s and is blocked on an unpaid debt named in §7.

**Caution measured at `41d1baa5e`**: `migrations_tests.rs` states that **the Postgres half of this
guard is executed by no test in that file** — the `PL/pgSQL` function was compared to the SQLite
triggers *by reading*. After this edit the Postgres arm's only execution remains the pg tier.

**Implements**: `cpt-cf-bss-products-flow-snapshot`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_entity_version`, `products_catalog_version_entry`
- Entities: `EntityVersion`, `CatalogVersionEntry`

### The request queue

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-request-queue`

The system **MUST** create `products_catalog_version_request` carrying `tenant_id`, `source`,
`lane ∈ {interactive, bulk}`, `request_key`, `operation_key` (**nullable** — the bulk batch
identity), `requested_at`, `state ∈ {pending, coalesced}` — two values, **P-D-60** having struck
`superseded` — and `satisfied_by_version_id`
(**nullable** FK to `products_catalog_version`). **Both value columns carry a roster**, on the
slice's own convention that *"every other state column in the set carries one"*: without one on
`lane`, the column ships as free text, the coalescer's two-lane branch has an unhandled third case,
and a typo'd lane produces a request neither window ever drains.

`request_key` **MUST** be `UNIQUE` with `(tenant_id, source)`. **The tenant column is part of the
key deliberately**: it is what the per-tenant coalescer selects on, and without it one `source`
serving many tenants collides across them.

`satisfied_by_version_id` **MUST** be a column rather than a parameterized state value
(**P-D-50**): without it a replayed `CatalogVersionPublished` cannot have its `satisfiedRequests`
set rebuilt, and pricing's stuck pending refs cannot be reconciled.

`requested_at` **MUST** be stamped by the door at ingress and **MUST NOT** be accepted from the
caller — `design/06` §1.7's entity requires it and the lane SLO measures from it.

**The writer is named** (**P-D-60**): the **increment transaction** marks every request it satisfied
`coalesced` and stamps its `satisfied_by_version_id` in the same transaction — it is the transaction
that produces the `satisfiedRequests` set, which is the set P-D-50 gave the column its existence to
let a replay rebuild. `coalesced` is **terminal**: a satisfied request is history naming its
satisfying version. And `superseded` is **struck rather than given a door**, because nothing
supersedes a request — a failed mechanical run re-coalesces and retries fresh, an unregistered source
is refused at the door before a row exists (**P-D-52**), and an idempotent replay is caught by the
UNIQUE above.

**Implements**: `cpt-cf-bss-products-flow-increment`

**Touches**:
- DB Table: `products_catalog_version_request`
- Entities: `IncrementRequest`

### The freeze ledger and the participant set

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-freeze-ledger-tables`

The system **MUST** create `products_freeze_participant`, the governed **live** registered set;
and `products_freeze_ack`, keyed `(tenant_id, catalog_version_id, participant)` →
`state ∈ {pending, acked, released, not_frozen(forced)}` — **four values, one column** — carrying
`acked_at`, `released_at` and `not_frozen(forced_at, ceremony_ref)`.

The **retention-release fact MUST ride its own column**, `released_at`, and **MUST NOT** become a
fifth state value. An earlier design revision wrote a `released(forced)` *state*, which asked one
column to hold two values and left the implementer choosing which of two requirements to break.

**Six transitions are admitted and no others** (**P-D-60**), stated in `design/06` §4:
`pending → acked`; `pending → released` and `acked → released` through the participant's own
`catalog_version × release` door, whose precondition is that it holds no live references rather than
that it acked; `pending → not_frozen(forced)` by force-completion, **missing participants only**, so
a row already `acked` or `released` is never overwritten; and `not_frozen(forced) → acked` /
`→ released` for a recovered participant. `released` is **terminal** and `released_at` is
**write-once** — a later ack does not clear it, the state moving to `acked` being what makes the stamp
inert, since slice 10's gate reads the `(state, released_at)` pair.
**The table has no entry point on purpose**: who writes `pending` at all is §7 row 46's, and the
question of what creates the ledger rows is `design/06` §6's, both open.

These rows **MUST NOT** be garbage-collected while their version exists: they are AC #44's
version-liveness source, and never the per-SKU reference count, which carries no version dimension.

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- DB Table: `products_freeze_participant`, `products_freeze_ack`
- Entities: `FreezeParticipant`, `FreezeAck`, `FreezeLedger`

### The increment-request contract, in `bss-products-sdk`

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-increment-request-port`

The system **MUST** publish the increment-request contract as a **client trait in
`bss-products-sdk`**, a typed contract a consumer resolves from `ClientHub` rather than an
implementation package, with the in-process binding as the default deployment mode. The contract
**MUST** carry the whole `IncrementRequest` — `(source, lane ∈ {interactive, bulk}, request_key,
operation_key?)`, `requested_at` being the door's — and **MUST** be idempotent per
**`(tenant_id, source, request_key)`**. The tenant column is stated here for the reason
`cpt-cf-bss-products-dod-request-queue` gives it in the storage key: without it one `source` serving
many tenants collides across them, and an SDK cache keyed on the pair alone would hand one tenant's
pending answer back to another.

**Its error taxonomy MUST separate "not wired" from "unreachable" from "unusable answer"** — the
axis a transport choice actually moves, since an in-process binding cannot fail with a network
error.

**What the lane SLO does not bound is the door's own answer time, and the two are different
objects** (**P-D-56**). The p95 ≤ 60 s of C1 measures `requested_at → published_at`, i.e. the
**version**. Pricing holds the **call** to `DEFAULT_REGISTRY_CALL_TIMEOUT_SECS`, and its own doc says
why that is short: *"what the budget bounds is not the registry's latency but the write transaction
the caller is holding open while it waits — ten of the twelve call sites are inside one."* Its
`infra::registry_deadline` seam exists for the same reason — *"a hung peer pins a transaction, its
row locks and a pool connection on every mutating path at once"*.

**So the door's synchronous path MUST have no unbounded step in it**: it stamps `requested_at`, claims
idempotently, enqueues and answers. It **MUST NOT** take the per-tenant lease — that is the
coalescer's, `design/06` §2 rule 2 giving it to the worker that *drains the queue* — and **MUST NOT**
resolve a committed version. **Five seconds is not the bound**: the consumer's
`registry_call_timeout_secs` is per-deployment configurable, rejects `0` and is capped at
`MAX_REGISTRY_CALL_TIMEOUT_SECS = 60`, so the door is held to the *smallest* value a consumer may
configure rather than to that default. *(An earlier revision of this paragraph put the lease inside
the door's synchronous work; a door that waited on a per-tenant lease could not fit any such budget
under contention, and the two claims could not both hold.)*

**This is the SDK's first write method**, and that is a shape decision this DoD names rather than
takes. `bss_products_sdk::api::ProductsClient` ships exactly two methods, `get_product` and
`get_sku`, and its own doc calls it *"the in-process contract for **reading** registry entities"*.
Whether the increment contract widens that trait or arrives as a second one is §7's.

**The counterpart port already ships and pre-agreed to become an adapter over this one.**
`bss_pricing_sdk::CatalogVersionRegistryV1` carries `request_version(ctx, request_id)` →
`PendingVersionRef { request_id, pending_ref }` and `committed_version(ctx, pending_ref)` →
`Option<CatalogVersion>`, with a fail-closed `UnconfiguredCatalogVersionRegistryV1` default and a
`LocalDevCatalogVersionRegistryV1` dev arm. Its module doc: *"when the registry publishes its own
SDK this trait becomes an adapter over it."* So the new dependency edge runs **`bss-pricing` →
`bss-products-sdk`**, not the reverse — the pricing-side doc is explicit that the contract lives
there only so *"the registry gear can implement it without depending on `bss-pricing`"*.

**Four measured mismatches between that port and this contract are §7's, not this DoD's**: the
missing `source`/`lane`/`operation_key` operands, the un-echoed `pending_ref`, the doorless
`committed_version` poll, and the shipped port's fourth error arm (`Rejected`, with its
`CATALOG_VERSION_REJECTED` wire constant) which this feature's roster has no counterpart for.

**Implements**: `cpt-cf-bss-products-flow-increment`

**Touches**:
- API: `POST /bss-products/v1/catalog-version-requests`
- Entities: `IncrementRequest`

### The increment-request door

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-request-door`

The system **MUST** serve `POST /bss-products/v1/catalog-version-requests` as the
**out-of-process binding** of the contract above and as the authz door **both** bindings pass
(S2S, `catalog_version × request`).

The trigger set **MUST** be exactly: registered downstream addressability requests (pricing); this
gear's own bulk commits from `09-bulk-promotion`, a registered **internal** requester whose
requests carry an `operation_key` so the batch coalesces into one version and which sends **no
close signal**, the window ending on the five-minute hard max (**P-D-46**); and the operator
**catalog-publish** act.

**An entity publish MUST NEVER enqueue an increment**, and a retirement's `effectiveAt` flip
**MUST NOT** either — the next demand-driven version reflects it.

**A request whose `source` is outside that set MUST be refused `REQUEST_SOURCE_UNKNOWN`**
(**P-D-52**). The refusal is raised **after** the `catalog_version × request` grant has passed, so it
is a precondition on the request's content rather than an authorization fact — and it **MUST** arrive
as a `FailedPrecondition` carrying a precondition violation of type `CATALOG_VERSION_REJECTED` with
the registry's own sentence as the description, because that pair is what the `pricing-sdk` port's
`Rejected` arm discriminates on. **A 403 would land on the port's `Other` arm** and leave the arm
unreachable, which is the asymmetry this code was minted to close.

**Implements**: `cpt-cf-bss-products-flow-increment`

**Touches**:
- API: `POST /bss-products/v1/catalog-version-requests`
- Entities: `IncrementRequest`

### The coalescer and its per-tenant lease

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-coalescer`

The system **MUST** drain the queue with **one worker per tenant**, coalescing interactive requests
within ≤ 5 s of the earliest pending, and holding a **keyed bulk batch open** until the
five-minute hard max from its earliest request — landing as **one** version, with interactive
versions free to publish in between without shredding it. D-47's *"bulk … coalesces into one
version"* holds **per `operation_key`**, not per quiet window.

**The lease belongs to the drain worker, never to the door** (**P-D-56**): the request door enqueues
and answers, and single-activeness is held by the per-tenant worker that drains. So the ≤ 5 s window
and the five-minute bulk maximum are **inputs to the lane SLO, not the door's answer time**.

Single-activeness **MUST** be taken through `gears/bss/libs/coord`'s `LeaseManager` rather than a
hand-rolled advisory lock: it is the shared BSS primitive, both `bss-pricing` and `bss-ledger`
already depend on it, and `LeaseGuard::with_ack_in_tx` is the write fence a serialized increment
transaction needs. `products/Cargo.toml` does **not** carry the dependency today and **MUST** gain
it — **and the gear's `Migrator` MUST register `coord::migration::Migration::in_schema("bss")`**,
as `bss-pricing`'s and `bss-ledger`'s migrators already do. The dependency alone compiles and then
fails at runtime on every increment against a `coord_leases` table no migration in this gear
creates.

**The lease key is per tenant, and that shape is already in production in BSS** —
`bss-ledger` keys `recognition-run:{tenant}:{period_id}` and
`period-close:{tenant_id}:{legal_entity_id}:{period_id}`, and `coord`'s README names one per
`(tenant, period)` as a typical fit. Pricing's *"per gear and per pass, never per tenant"* is a doc
comment on **one of its three sweep keys** and carries a reason specific to a sweep: *"one sweep is
one pass over every tenant, so there is nothing per-tenant to hold."* The **cardinality cost** is
what §7 registers.

**A steady interactive trickle MUST NOT defer a bulk window past its hard max**; the deadline logic
gets its own probe.

**Implements**: `cpt-cf-bss-products-algo-cv-concurrency`

**Touches**:
- DB Table: `products_catalog_version_request`, `coord_leases`
- Entities: `IncrementRequest`

### The snapshot builder and the first row collection through the canonical pin

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-snapshot-builder`

**`inst-sn-collect`'s "serialized transaction" is the coalescer's serialization, not a database
isolation level** (**P-D-53**): one worker per tenant is what serializes, and the transaction itself
opens at the engine default.

The system **MUST** collect, **inside the serialized transaction**: every published or deprecated
entity's current published-version **reference** into `products_entity_version`; and, as **stored
canonical copies**, the current categories with their live display values, the current attribute
definitions, **the category values**, the per-entity metadata maps as of that instant (**P-D-06**),
the recognized sets, the
**freeze-participant set snapshot** (AC #23) and the **reference-producer set snapshot** — 07's
symmetric ride, for which the capture store declares its own `capture_kind`.

**Seven, and the seventh is §4's.** `design/06` §4's capture-store bullet carries `category values`
as a kind of its own; the normative instruction steps `inst-sn-collect` and `inst-df-diff` both list
**six** and omit it. §4 governs on a column-level fact and a `capture_kind` value is one, so the
roster here is seven — and the divergence is registered in §7 as owed to those two steps rather than
resolved here.

**Every live capture MUST be a stored copy, never a reference.** Category values, metadata,
recognized sets and both set snapshots have no frozen versions of their own, so a reference to
their live rows would break byte-identity the moment they moved. Only the Product and SKU halves
are references, into immutable rows.

The system **MUST** canonically serialize the manifest and checksum it through the gear's **one**
rendering rule — `domain::canonical`, not a second answer beside this builder.

**This feature is the first row collection to reach that module, and it brings two arms that module
already records as owed there rather than at a call site.** `domain::canonical`'s own doc:

> *"§4.3 sorts a *row collection* — the category-assignment set, the attribute-value set — **by the
> collection's own identifier**, and no payload on this surface carries a collection today… The
> first door whose payload carries a collection owes that sort **here**, rather than at its own call
> site…"*

and, on the complete-set mode:

> *"The roster applies to the **outermost** object only. … a nested complete set arrives with the
> first row collection, and is owed with the collection sort."*

So the **location** of both arms is already settled by shipped code, and **MUST NOT** be
re-litigated at the builder. What is **not** settled is the sort **key** — **P-D-28** orders
*fields, not rows*, and **P-D-29**'s row rule is scoped to the category-assignment and
attribute-value sets *inside the content*, so the manifest's entry rows and capture rows have no
named key. Until one exists, two runs or two engines may hash the same snapshot differently. §7.

**Implements**: `cpt-cf-bss-products-flow-snapshot`

**Touches**:
- DB Table: `products_catalog_version_entry`, `products_entity_version`
- Entities: `SnapshotBuilder`, `VersionManifest`

### Stage-vs-commit re-validation, both arms

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-stage-commit-revalidation`

**The transaction opens at the engine default — `READ COMMITTED` on Postgres** (**P-D-53**), and the
guard below is what closes the race, not the isolation level. The builder records each collected
entity's `(id, published_version, lifecycle_state)` and re-reads the heads before commit; any
difference refuses `STAGED_ENTITY_CHANGED`. A build relying on the snapshot instead has no detector:
under a snapshot-isolating level the re-read returns the collect-time view and the guard cannot fire,
and under `SERIALIZABLE` the transaction aborts rather than raising the code §6 requires.

The builder **MUST** record each collected entity's `(id, published_version, lifecycle_state)` and,
before commit, re-read the heads. Any entity whose published version **or lifecycle state** moved
between collect and commit **MUST** fail the run closed.

**Both arms are obliged, and the second is the one a version-only check misses**: AC #40's `When`
names the `deprecate`/`retire` race explicitly, and a transition writes **no** version row, so an
entity can move state with its `published_version` unchanged.

The **lane split is normative** (**P-D-09**): an operator publish raises `STAGED_ENTITY_CHANGED`
**naming the entity**; a mechanical run re-coalesces and retries fresh, and **the request is never
lost**.

**Implements**: `cpt-cf-bss-products-flow-snapshot`

**Touches**:
- DB Table: `products_catalog_version_entry`, `products_product`, `products_sku`
- Entities: `SnapshotBuilder`

### Version binding at freeze

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-version-binding`

When a bound-not-yet-frozen reference re-resolves to a newer version, the **resolve or finalize
response itself MUST carry `(boundVersion, resolvedVersion, diffRef)`** — the diff is surfaced
**to** the module, not left for it to know to pull. The `CatalogVersionPublished` changed-entity
list and the AC #20a diff surface back it for arbitrary spans.

The consuming module's **duty to act** on the diff is booked in `12-consumer-contracts`'
`ObligationRegister` as **owed**, and is not yet a fixture. This DoD obliges the surfacing only.

**Implements**: `cpt-cf-bss-products-flow-snapshot`

**Touches**:
- Entities: `VersionManifest`

### The intentful resolver and byte-identity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-intentful-resolver`

The system **MUST** serve resolution through one component spending **`catalog_version × read`**
(the pair `design/06` §2 puts on `IntentfulResolver`) that requires `intent`, and that component
**MUST** be the single raising door of `CATALOG_VERSION_UNKNOWN` for **both** resolve and diff. Absent `intent` is `INTENT_REQUIRED`. `browse` serves any published version at once. `posted`
is refused per §2's error scenarios.

Re-resolution **MUST** be byte-identical forever: content renders from the **stored manifest** plus
the frozen entity versions and is **never re-collected**, and the checksum is returned and
verifiable.

**This resolver is one of five doors of this feature with no route declared anywhere in the design
set.** The increment door and the diff carry one each; `08-read-models` puts this surface out of its
own scope, and `01-foundation` hands this feature the intent clause without a surface. The other
four are named by `05-governance` §3.2's own `Doors` column: `catalog_version × ack` and
`× release` (*"S2S, no route declared"*), `catalog_version × force_complete` (*"has no route
declared — an operator surface named in prose only"*) and `freeze_participant × write` (*"no route
declared"*). §7 carries all five; row 12 is the slice's own carry about this one. The
route is therefore **owed and not invented here** — §7.

**Implements**: `cpt-cf-bss-products-flow-resolve`

**Touches**:
- DB Table: `products_catalog_version`, `products_catalog_version_entry`, `products_freeze_ack`
- Entities: `IntentfulResolver`, `VersionManifest`

### The ack door

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-ack-door`

The system **MUST** record participant acks in `products_freeze_ack`, idempotently per
**`(tenant_id, catalog_version_id, participant)`** — the table's own primary key at `design/06`
§4 — and **MUST** accept a participant's ack **only from that participant's own service identity**
(S2S claims).

**The full key is stated rather than the slice's shorthand.** `inst-fz-ack` writes the idempotency
as *"idempotent per `(version, participant)`"*, which elides the tenant; because
`catalog_version_id` is monotonic **per tenant** rather than globally, that pair is not unique
across tenants and an ack door keyed on it would accept one tenant's ack against another's version
number. §4's primary key governs on any column-level fact, and it carries `tenant_id`.

`freezeComplete` **MUST** be evaluated against **that version's snapshotted set**, never the live
registered set. An ack from a principal outside it is `PARTICIPANT_UNKNOWN`, refused and audited —
a **membership** check, not authentication.

Acks and re-triggers are **audit-plane**: this DoD obliges an audit row and **explicitly no broker
event**, the ack door being inbound.

**Two vacuity problems are open and are not closed by this DoD.** Nothing in the design set creates
the ledger rows other than consumption of `CatalogVersionPublished`, which is emitted **after** the
increment transaction commits — so in that window an entirely unfrozen version's `posted`
resolution succeeds, against C5 and AC #21. And `freezeComplete` defined over the ledger's
**current** value **regresses** when a participant releases, flipping a version back out of
posting-safe. Both are §7's; the operand for the first exists (**P-D-49** gave
`10-retention-erasure` this version's `participant_set_snapshot` for the same vacuity).

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- DB Table: `products_freeze_ack`, `products_audit_log`
- Entities: `FreezeAck`, `FreezeLedger`

### The bounded timeout, and the export it owes `01-foundation`'s config

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-freeze-timeout`

The timeout **MUST** be configured, **MUST** fail **closed** — past it the version stays
non-posting-safe — and `freeze_overdue` **MUST** name the silent participants. In v1 that is
pricing, the registered set's one member (**P-D-48**), so the PRD §15 open is visible in this gear's
own telemetry from day one.

**The timeout's maximum MUST be exported as the second operand of `01-foundation`'s idempotency
retention floor, and the shipped code has the hole shaped for it.** `config.rs` names this feature
**twice**:

> *"The retention floor `design/01-foundation.md` §3.2 `inst-fd-idem-retention` and C6 pin:
> `max(24h, max_freeze_timeout)`, **whose second half has no source until the catalog-version
> feature exports it**."*

and, on the field itself:

> *"The floor the design pins is 24 hours **and** at least the maximum freeze timeout, **which the
> catalog-version feature exports. Until that feature exists the second half has no source**, so
> this carries the first."*

So `ProductsConfig::resolved_idempotency_retention_hours`'s `clamp` **MUST** take
`max(IDEMPOTENCY_RETENTION_FLOOR_HOURS, max_freeze_timeout)` as its lower bound rather than the
constant alone. The design set already states the obligation in the other direction —
`inst-fz-timeout` says the timeout's *"value floors 01's idempotency retention"* — so the design and
the code agree and only the wire is missing.

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- DB Table: `products_freeze_ack`
- Entities: `FreezeLedger`

### Force-completion, and the gate subject it cannot form

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-force-completion`

Force-completion **MUST** be a `05-governance` two-person ceremony on
`catalog_version × force_complete`, `N`-governed, recording `quorumReduced` on the record **and on
`FreezeForceCompleted`** below the default of 2 (**P-D-13** — no fixed floor, since one would leave
a solo tenant's timed-out version permanently un-resolvable, the class of block **P-D-11** exists to
remove).

It **MUST** record each missing participant as `not_frozen(forced)` **and** stamp `released_at` on
that same registration in the same transaction, while its `state` stays `not_frozen(forced)`. **The
stamp is meaningful only while that state holds**: a forced participant that later recovers and
acks moves to `acked`, and `10-retention-erasure`'s gate reads the `(state, released_at)` **pair**,
so the stale stamp frees nothing.

The reason the stamp exists at all is measured: a participant recorded `not_frozen` never acked and
**cannot use the S2S release door**, which runs under its own identity, so a retention gate
requiring *every* registration to read `released` would hold a force-completed version
un-collectable forever. A participant that froze nothing holds no live references to that version
**by construction**, so the release is a statement of fact rather than a courtesy.

It **MUST** flip the version to `freeze_state = complete(forced)`, return the **per-participant**
frozen state, and leave **posted resolution refused** (`VERSION_FORCED_INCOMPLETE`) until every
forced participant freezes or releases through its own door. Refusals **MUST** ride 05's gate codes;
there is no `FORCE_COMPLETE_QUORUM`.

**The shipped gate cannot form this subject.** `domain::governance::GovernanceGate::evaluate` takes
an `EntityRef` whose `entity_kind` is `bss_products_sdk::models::EntityKind`, and that enum is
exactly `Product | Sku`. **A catalog version is neither.** This is not a missing operand but a
wrong-typed subject, and it applies equally to the participant-set DoD below.

**And the subject is not the only unformable argument.** `evaluate`'s second parameter is
`expected_revision: InternalRevision` — the door's `If-Match` (**P-D-33**), which the trait's doc
calls *"not advisory: an approval is only usable against the exact revision it pinned"*. A catalog
version and a participant set carry **no internal revision**, as this document states about the
event body below. Widening the subject type alone would still leave the call unwritable, so §7 names
both arguments and the answer arrives in one round rather than two.

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- DB Table: `products_freeze_ack`, `products_approval`
- Entities: `FreezeLedger`, `FreezeAck`

### Participant-set governance

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-participant-set`

Membership of `products_freeze_participant` **MUST** be a `GovernedLiveOp` on
`freeze_participant × write` — **material**, and enumerated as one of 06's kinds in
`05-governance`'s input (d). Each change **MUST** emit `FreezeParticipantSetChanged`, because
participants must learn they were added.

Each version **MUST** resolve `freezeComplete` against **its own** `participant_set_snapshot`
forever: a removal after publish **MUST NOT** retro-flip a historical version (AC #23).

**`participant_set_snapshot` is stored twice and only one copy is inside the checksum** — the slice
puts it on the `products_catalog_version` row *and* in the capture store, whose bullet says the
checksum covers both halves — and `cpt-cf-bss-products-dod-version-entry-table` obliges exactly
that for the capture half, so the **capture** copy is inside the checksum. What is stated nowhere is
which of the two copies is authoritative, and therefore whether the
`products_catalog_version` row's copy is inside it. §7.

The gate-subject problem from the DoD above applies here identically.

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- DB Table: `products_freeze_participant`, `products_catalog_version`
- Entities: `FreezeParticipant`

### Liveness records and the release door

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-liveness-and-release`

The per-`(catalogVersionId, participant)` registration and ack rows **MUST** be the
**version-liveness source** (AC #44) — never the per-SKU reference count, which carries no version
dimension.

Liveness **MUST** end by an **explicit release**: the `catalog_version × release` door (S2S, the
participant's own identity) records that the participant holds no more live references to that
version. This is the second half of PRD §9.2's freeze-participant contract (**P-D-18**), which had
been a duty on three counterpart gears that §9.2 told none of them they owed.

Version-liveness **MUST** be evaluated over the version's `participant_set_snapshot`, so that a
snapshot member with **no registration row** still holds the version (**P-D-49**).

**The honest v1 posture MUST be recorded rather than glossed.** The v1 set's one participant,
pricing, is §15-silent, so every version's registration, **where one exists at all**, sits
`pending` — a state the summary formula *"acked-and-not-yet-released"* does not classify, since it
presumes an ack. (§7 row 7 records that nothing in the design set creates the row in the first
place, so the two facts are separate and both open.) **The operative
predicate is the retention gate's**: `inst-rt-gc` and the PRD require every registration to read
`released`, or `not_frozen(forced)` with `released_at` stamped, so a `pending` registration holds
the version. The gate therefore over-retains and never over-collects — the fail-safe direction —
and **MUST** be read as *designed and not yet exercised* rather than as a working reclamation path.
Whether the formula here and in P-D-18 should be restated to match the gate's predicate is the
owner's.

**Implements**: `cpt-cf-bss-products-flow-freeze`, `cpt-cf-bss-products-flow-grandfathering`

**Touches**:
- DB Table: `products_freeze_ack`
- Entities: `FreezeAck`, `FreezeLedger`

### The grandfathering invariant, made auditable

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-grandfathering`

A frozen snapshot referenced by a grandfathered consumer **MUST** never be mutated, and the system
**MUST** hold this **by construction** rather than by a check: entity versions are append-only
under 01's history rows and their trigger; manifests are append-only under this feature's own
guard; and retirement and deprecation touch **head rows only**.

The DoD exists so the delegation is auditable from the registry side: **eligibility policy is
plan-price's and subscriptions-lifecycle's, the immutability is this gear's.**

**Implements**: `cpt-cf-bss-products-flow-grandfathering`

**Touches**:
- DB Table: `products_entity_version`, `products_catalog_version_entry`
- Entities: `VersionManifest`

### The `compositionPending` clearing lane

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-composition-clear`

On the inbound composition signal — pricing's, **unregistered on their side** (PRD §15) — the
system **MUST** clear `composition_pending` as a **system save plus a re-publish** of the head at
version N+1, carrying **no** uncomposed-bundle override so **P-D-30**'s predicate is false on it and
the flag is not re-raised.

It **MUST** emit `SkuCompositionCleared` **and** the publish's own `SkuPublished` — two events, both
naming the same `publishedVersion` (**P-D-60**, and `cpt-cf-bss-products-dod-cv-events` carries the
reason).

The flag **MUST** be written by the **publish door's own head-row `UPDATE`** — the statement
carrying `published_version += 1` — and by no other, since 01 §4.2 admits the change only there and
a save never bumps the version (**P-D-32**). Verified in shipped code: `composition_pending` is a
`products_sku` column and `infra::storage::repo`'s publish twin takes it as a **parameter**, its own
doc stating that the flag *"rides this statement and can ride no other"*.

It **MUST** require a **clean head**. A publish freezes the *full* entity content
(`inst-fd-publish-txn`), so this publish cannot deliver what `05-governance` calls it — one *"whose
sole content is a system-owned flag"* — while the head carries anything else; any unpublished local
edit or open approval would ride out under an `ApprovalRecord` with **no human approver**.

On a dirty head the clear **MUST** be **deferred, never refused** (**P-D-14**, confirmed by
**P-D-48**): the signal is durable and idempotent, the flag stays `true`, `composition_clear_held`
names the entity and the blocking edit or approval, and the clear re-evaluates when the head next
goes clean — including immediately after the operator publishes their own edit through the ordinary
gate. **The signal is never dropped and never carries someone else's change.**

On a clean head it is **not exempt from the gate**: it runs as a `system_signal` approval subject
auto-satisfied by **the signal itself as the authorizing principal**, recorded on the
`ApprovalRecord` with the signal reference — the approver being the governed pricing-side act, named
and audited, rather than an exemption. That satisfaction **MUST** be independent of the tenant's
configured `N` (**P-D-11**).

It **MUST** emit `SkuCompositionCleared` — **this gear's outbound event, distinct from the inbound
`BundleCompositionCompleted` that drove it**; a registry emitting the very event it consumes is a
loop, not a contract. Prior frozen versions **MUST** keep the flag as it was (C4).

**Implements**: `cpt-cf-bss-products-flow-composition-clear`

**Touches**:
- DB Table: `products_sku`, `products_entity_version`, `products_approval`
- Entities: `Sku`, `EntityVersion`

### The diff door

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-diff-door`

The system **MUST** serve `GET /bss-products/v1/catalog-versions/{a}/diff/{b}`, spending
**`catalog_version × read`** through the same shared version-lookup component as resolve, covering
**every snapshot member**: entities added and removed, per-entity published-version deltas rendering 01's
history diff, **and the capture half** — all seven capture kinds: category tree and display values,
attribute definitions, category values, recognized sets, per-entity metadata maps, and the
participant and producer sets. A metadata-only or live-entity-only change between two versions
**MUST** appear; the manifest's own membership is the diff's universe (AC #20a).

It **MUST** be computed read-only from the two **stored** manifests, **MUST** be byte-stable for a
given pair, and **MUST** have **no retention effect**.

**Implements**: `cpt-cf-bss-products-flow-diff`

**Touches**:
- API: `GET /bss-products/v1/catalog-versions/{a}/diff/{b}`
- DB Table: `products_catalog_version_entry`
- Entities: `VersionManifest`

### The error taxonomy, wired into `DomainError`

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cv-error-taxonomy`

The system **MUST** add a `DomainError` variant for each of the **seven** codes in §3, and each
**MUST** carry its wire code through `DomainError::code`.

**The extension is compile-gated, and that is the good news measured here.** `DomainError` ships
14 variants and `code()` is *"deliberately exhaustive rather than a catch-all: a variant added
without a code is a compile error here, which is the only thing that keeps the taxonomy and the
vocabulary from drifting."* **None of the seven exists today.**

Each **MUST** carry an RFC 9457 problem response at the status §3 states, following the sibling
pricing gear's mapping code by code: **422** for content the door cannot process, **409** where the
current state refuses the act, **403** where the caller may not perform the act at all, **404**
only where a path segment names a resource this tenant has none of.

`PARTICIPANT_UNKNOWN` **MUST** be **403** rather than 404, because the caller's identity is the
subject of the refusal and a 404 would leak whether the version exists.

**`REQUEST_SOURCE_UNKNOWN` MUST NOT be 403, and its status is the one exception to the ladder
above** (**P-D-52**): it is a `FailedPrecondition` — 422 architecturally, 400 on the wire — and
**MUST** carry a precondition violation of type `CATALOG_VERSION_REJECTED`, because that is what the
consumer's port discriminates its refusal arm on. The grant has already passed when it is raised, so
it is not an authorization refusal despite naming a caller's source. **This is the only code in the
gear whose wire shape a consumer fixes**, and it is stated here so a status sweep does not align it
with `PARTICIPANT_UNKNOWN`.

**`INTENT_REQUIRED` is a 422 architecturally and reaches the wire as a 400** carrying its code —
not because *no path can produce* a 422, which `design/01-foundation.md` §3.3 records as a retired
false premise, but because this gear declares no transport override anywhere and neither does
pricing, so every registry code has exactly one wire shape. A bare 400 stays reserved for a malformed request.

`INTENT_REQUIRED` **MUST** be added to
`error_mapping_tests::the_products_owned_422_codes_stay_wire_400_by_design`'s array in the same
change. That test is a hard-coded three-element list, so a fourth architectural 422 is unguarded
until its row lands — and the day someone attaches `Http::status_code(422)` to it, every test stays
green.

**Every new variant MUST carry a resource marker** rather than falling to `ProductResource`'s
default. `infra::error_mapping`'s own rule is *"Two resource markers, not one"* — a `Problem`'s
`resource_type` and a caller's authorization both key on which resource actually refused — and
`error_mapping_tests` pins `ProductResource` as the default for every unclaimed variant. Without a
marker per new authz label, six catalog-version and freeze refusals would reach the wire naming a
**Product that refused nothing**. Which marker each code takes follows the labels
`cpt-cf-bss-products-dod-cv-authz` declares; where the design set does not say, §7 carries it.

**Implements**: `cpt-cf-bss-products-algo-catalog-version-errors`

**Touches**:
- Entities: `CatalogVersion`, `FreezeAck`

### The authz surface, and the four rosters it reddens

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cv-authz`

The system **MUST** declare two new authz labels — `catalog_version` and `freeze_participant` —
and the action constants `request`, `ack`, `release` and `force_complete`, and **MUST** register a
permission instance per `(resource_type, action)` pair the RBAC catalog of `05-governance` §3.2
grants on these resources. Measured against that roster: **six actions on `catalog_version`**
(`request`, `ack`, `release`, `read`, `force_complete`, `publish`) and **one on
`freeze_participant`** (`write`).

**Four shipped roster tests bound this, and one of them is positional.** Each **MUST** be updated
in the same change, and the DoD names them as lines because a blanket criterion is ticked by
inspection:

- `authz_tests.rs` asserts `labels::ALL == [labels::PRODUCT, labels::SKU]` — a **positional**
  equality, so a third label reddens it.
- `gts::permissions`' `EXPECTED_PERMISSION_IDS` holds exactly six ids and is compared as a **set in
  both directions**, with a separate length check that catches a duplicate registration.
- `catalog_resource_types_match_authz_labels_all` asserts the catalog's distinct `resource_type`s
  are **exactly** `labels::ALL` — a label added to one and not the other fails here.
- `catalog_actions_are_declared_action_constants` compares each instance's action against a
  **hard-coded** `known = [READ, WRITE, PUBLISH]`, so **every new action must be added to that array
  as well as to `actions`**.

The system **MUST** also declare one `crate::authz::resource_types` descriptor per new label.
`authz.rs` requires it in terms — *"every authoring door passes one of these, never a bare label
string, to `access_scope`"* — and **nothing goes red if it is omitted**: the descriptor test asserts
only the two that exist, and the stub type-schemas derive from `labels::ALL`. So the labels would
land, the permission instances would land, all four rosters would go green, and the six doors would
have no `ResourceType` to hand the gate.

Both modules admit the extension: their docs say the wider catalog *"belongs to the slices that
build those doors"*. So this is a cost to state, not a contradiction to resolve.

**`catalog_version × publish` is granted by 05's roster and consumed by no door this feature
declares** — its operator lane goes through the request door, and *"an entity publish NEVER
enqueues an increment"*. Either the roster grants an action no route spends or this feature is
missing a door; **this DoD does not decide** — §7.

Separately, `crate::authz::authz_label_type_schemas` is **still unregistered** from `Gear::init`,
which `authz.rs` records as owed to *"the slice that adds the first authoring door"*. That is
`01-foundation`'s debt, not this feature's, and is named so it is not paid twice.

**Implements**: `cpt-cf-bss-products-flow-increment`, `cpt-cf-bss-products-flow-freeze`

**Touches**:
- API: `POST /bss-products/v1/catalog-version-requests`
- Entities: `CatalogVersion`, `FreezeParticipant`

### The four events, and the body shape none of the shipped types has

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cv-events`

The system **MUST** emit exactly four broker events: `CatalogVersionPublished` (carrying the
changed-entity list, `satisfiedRequests`, the checksum and the participant set),
`FreezeForceCompleted`, `FreezeParticipantSetChanged` and `SkuCompositionCleared`. Acks and
re-triggers are audit-plane and **MUST** carry **no** broker event.

Each **MUST** be enqueued in the **same transaction** as the act it announces, and each **MUST**
carry a **versioned** schema reference.

**`SkuCompositionCleared` rides beside the `SkuPublished` the clear's own publish emits, and that
publish event MUST NOT be suppressed** (**P-D-60**): `inst-fd-publish-emit` fires unconditionally and
`08`'s projector keys on `publishedVersion` from `*Published`, so suppressing it would leave the read
model a version behind on the entity whose flag just changed. The two are **additive**, carry the same
entity and the same `publishedVersion`, and a consumer keyed on version therefore sees **one** version
change — so no consumer obligation is created. `09`'s additivity rule is **not** widened by this; it
stays scoped to its coalesced summary.

**Two hand-transcribed rosters pin the eight, and all four events redden both — each MUST be
extended in the same change.** `infra::events::events_tests::THE_EIGHT` pins the payload roster
exact in both directions plus a length assertion; `infra::broker::broker_tests::THE_EIGHT` is a
second, wider roster — one row per event carrying the payload token, the `TYPE_ID` it must map to
and the `SUBJECT_TYPE` that id must carry — with its counterpart `declared()` list hand-written
beside it. **Neither is derived from the code**, both for the same stated reason: *"a list built
from the code under test could only prove the code equals itself."* So the second one stays green at
eight-and-eight while four new events' `TYPE_ID`, `SUBJECT_TYPE` and `TOPIC` literals go
unasserted. The both-ways
sweep is deliberate: *"a missing entry is an event that would be refused at its first enqueue; a
*surplus* entry is a schema reference announced for an event this gear does not emit, which a
consumer contract would take for a promise."*

**Three of the four have no body type that fits, and this is stronger than a missing field.**
`EventBodyCore` is `{tenantId, entityKind, entityId, internalRevision, lifecycleState}` and the
module doc argues **against** a sixth field on it — *"would satisfy the two publish events and break
the other six … would have to invent a value for it"*. `EntityKind` is exactly `Product | Sku`. A
**catalog version** and a **freeze participant set** have no entity kind, no entity id, no internal
revision and no lifecycle state, so `CatalogVersionPublished`, `FreezeForceCompleted` and
`FreezeParticipantSetChanged` need a body with **no entity dimension at all**. Only
`SkuCompositionCleared` fits the existing shape, on a SKU.

**And three artifacts are missing, not one.** Beyond the body core, `TypedEvent` demands `TYPE_ID`,
`TOPIC` and **`SUBJECT_TYPE`** as compile-time constants, and every shipped event's `subject()`
returns its `entity_id` — with exactly two subject-type constants declared, one per entity. On the
interim queue `infra::events::enqueue` additionally requires an **`aggregate_id`**, which
`partition_for` consumes. So a subject that is not an entity needs a subject type, a value for
`subject()` **and** an aggregate id; two implementers left to invent the last would spread one
tenant's versions across partitions differently. All three are §7's.

**Implements**: `cpt-cf-bss-products-flow-increment`, `cpt-cf-bss-products-flow-freeze`,
`cpt-cf-bss-products-flow-composition-clear`

**Touches**:
- Entities: `CatalogVersion`, `FreezeAck`, `FreezeParticipant`, `Sku`

### Delivery is a precondition of this feature, not a config note

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-require-broker`

A deployment running this feature **MUST** set `ProductsConfig::require_broker = true` **once a
`dyn EventBrokerApi` provider is registered in its `ClientHub`** — and **MUST NOT** set it before,
because until a provider exists the setting makes the gear un-bootable everywhere, which the
`config.rs` sentence quoted below states outright. The ordering is part of the obligation, not a
caveat on it.

**This feature is the first whose own correctness depends on outbound delivery.** The freeze
protocol begins with participants **receiving** `CatalogVersionPublished`; with no broker, the
holding processor accumulates the events undelivered, no participant ever acks, and **every version
is posting-unsafe forever** — while the only signal is one `warn!` line, which
`config.rs` says is *"indistinguishable from a broker the gear failed to reach"*.

Measured at `c081872ab`: the **only** `register::<dyn EventBrokerApi>` anywhere is in
this gear's own `broker_tests.rs`, and `require_broker` defaults `false` because — in the config
doc's own words — *"as of 2026-08-30 no gear in this workspace registers a `dyn EventBrokerApi` in
any `ClientHub`, so defaulting to `true` would make this gear un-bootable everywhere today. The
default is expected to invert the moment a provider exists."*

So this DoD **MUST NOT** be read as flipping the default, and **its boot half already ships**:
`gear::BssProductsGear::init` carries `anyhow::ensure!(!cfg.require_broker, …)` on the fallback arm,
refusing to boot *"into the holding processor, which would accumulate every catalog event
undelivered"*. What this DoD adds beyond that is the **deployment** posture — the manifest or
config default that actually carries `require_broker = true` for a deployment running this feature.
**That artifact is not in this repository**, which is stated so the checkbox is not read as an
implementation obligation already met.

**Implements**: `cpt-cf-bss-products-flow-freeze`

**Touches**:
- Entities: `CatalogVersion`

### The posting-safe meters

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-posting-safe-observability`

The system **MUST** instrument `requested_at → published_at` at p95 ≤ 60 s and max 5 min, and
**MUST** raise `catalog_version_overdue` for a pending request past the lane deadline — the
registry-side mirror of pricing's `commit_overdue`.

**Those two numbers are the published batching SLO** (**P-D-56**), and that is the referent the
shipped consumer contract means: `committed_version`'s doc says *"A pending ref that stays unresolved
past the batching SLO is an alarm, not an error here — the caller decides that"*. Nothing new is
minted; the consumer's alarm and this meter **MUST** key on the same pair. It **MUST** expose `event → ack` per participant
from this ledger, and the gauges: pending-request age per lane and unacked participants per version.

**The `commit → durable-acceptance` meter is attributed to `01-foundation` by §3.3 and is declared
by no slice**, so the composite this DoD calls derivable from three meters currently rests on two.
Named, not assigned — §7.

**Implements**: `cpt-cf-bss-products-algo-posting-safe`

**Touches**:
- DB Table: `products_catalog_version_request`, `products_freeze_ack`

### The audit trail for this feature's acts

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cv-audit`

Every ack, release, re-trigger, force-completion, participant-set change and composition clear
**MUST** write an audit row through `01-foundation`'s audit plane, and a **refused** ack
(`PARTICIPANT_UNKNOWN`) **MUST** be audited as well as refused.

A force-completion's row **MUST** carry its `ceremony_ref`, the same value
`products_freeze_ack.not_frozen(forced_at, ceremony_ref)` stores, so the ceremony and the
registration are joinable from either side.

**`products_audit_log` has no column that can carry it.** Its shipped columns are `audit_id`,
`tenant_id`, `actor_ref`, `action`, `subject_kind`, `subject_id`, `subject_revision`, `error_code`,
`attempted_key`, `reason`, `correlation_id`, `written_at`, `session_id`, `seal_state`, `chain_id`,
`seq`, `prev_hash`, `row_hash` — no `ceremony_ref` and no generic payload column, and the token
appears nowhere in the crate. The column **MUST** therefore land **by editing
`m20260829_000004_create_products_audit_log.rs` in place**, on both engines, **nullable** since only
force-completion rows carry it — the same in-place discipline
`cpt-cf-bss-products-dod-referential-delete-predicate` follows. Overloading `attempted_key` or
`reason` with a ceremony id is not available: neither is joinable and neither is typed. Whether the
column is this feature's to add or `01-foundation`'s is registered in §7.

**Implements**: `cpt-cf-bss-products-flow-freeze`,
`cpt-cf-bss-products-flow-composition-clear`

**Touches**:
- DB Table: `products_audit_log`, `products_freeze_ack`
- Entities: `FreezeAck`

## 6. Acceptance Criteria

**The byte-identity flagship**

- [ ] Publish a version, then mutate everything mutable — heads, metadata, categories, recognized
      sets — then re-resolve the old version: the checksum is unchanged. This extends the 02
      **P-D-06** metadata probe to the full manifest.
- [ ] A re-resolution renders from the stored manifest and issues **no** collect query, asserted by
      observing the queries rather than by inspecting the result.

**Gapless serialization**

- [ ] Concurrent increment requests on one tenant produce sequential ids with no gaps and exactly
      one worker — driven by **real concurrency**, not read-then-assert.
- [ ] A run refused by re-validation consumes **no** id: the next successful version's id is the
      refused run's, not its successor.

**AC #40, both arms**

- [ ] An entity **re-published** between collect and commit: the operator lane fails
      `STAGED_ENTITY_CHANGED` naming the entity.
- [ ] An entity **retired or deprecated** between collect and commit — its `published_version`
      unchanged — fails the same way. This is the arm a version-only check misses.
- [ ] The mechanical lane on the same race retries fresh and the request survives with its
      `(source, request_key)` intact.

**The freeze protocol**

- [ ] Timeout reached: `posted`-intent resolution is refused, `browse` is unaffected, and
      `freeze_overdue` names pricing.
- [ ] Force-completion records `not_frozen(forced)`, stamps `released_at` on the same registration,
      flips `freeze_state` to `complete(forced)`, and `posted` resolution of that version is refused
      `VERSION_FORCED_INCOMPLETE` **naming each** `not_frozen(forced)` participant.
- [ ] That refusal is lifted **only** by that participant acking or releasing through its own
      `catalog_version × release` door — **nothing else lifts it**, and in particular the
      `released_at` the ceremony stamped does not.
- [ ] A historical version re-resolves `freezeComplete` against its **snapshotted** set after a
      membership change (AC #23).
- [ ] An ack under a service identity other than the participant's own is refused.

**Lane SLOs**

- [ ] Under a bulk burst: one version, ≤ 5-minute delay, and the interactive deadline honoured in a
      mixed window.
- [ ] A steady interactive trickle does not defer a bulk window past its five-minute hard max.

**Composition clear**

- [ ] A prior frozen version keeps `compositionPending = true` while the new version reads false.
- [ ] The clear survives replay — idempotent per signal reference.
- [ ] On a dirty head the clear is **deferred**: the flag stays `true`, `composition_clear_held`
      names the blocking edit or approval, and the clear completes on the next clean head without
      the signal being re-sent.

**The diff**

- [ ] A metadata-only change between two versions appears in the diff.
- [ ] A live-entity-only change — a category display value, a recognized-set member — appears in the
      diff.
- [ ] The diff is byte-stable for a given pair and creates no freeze-registration row.

**The referential predicate**

- [ ] Deleting a `products_entity_version` row that a `products_catalog_version_entry` **still
      references** is refused **by the guard**, on **both** engines — the probe passing even when the
      GC is bypassed entirely.
- [ ] Deleting a `products_entity_version` row that **no** entry references is admitted, on both
      engines. This arm is what proves the predicate is a predicate and not a renamed unconditional
      refusal.
- [ ] `UPDATE` on a frozen row is still refused with the **UPDATE** message, asserted apart from the
      delete message.
- [ ] On `products_catalog_version`, a `DELETE` and an `UPDATE` of any column other than
      `freeze_state` are each refused **with their own message, asserted apart**, on both engines;
      an `UPDATE` that moves **only** `freeze_state` is admitted. The admitted arm is what proves the
      whitelist is a whitelist and not a renamed unconditional refusal.

**Positive controls, one line per declared code** — seven codes, seven lines. A blanket criterion
here is ticked by inspection.

- [ ] `INTENT_REQUIRED` — a resolution request with no `intent` is refused; the same request with
      `intent = browse` succeeds.
- [ ] `REQUEST_SOURCE_UNKNOWN` — a request from a source outside the trigger set is refused **after**
      the grant passes, and the refusal carries a `CATALOG_VERSION_REJECTED` precondition violation;
      the same request from a registered source succeeds. **Both halves in one probe**: a refusal that
      omits the violation type is invisible to the consumer's `Rejected` arm, which is the whole
      reason the code exists.
- [ ] `FREEZE_INCOMPLETE` — `posted` against an open ledger is refused; the same request after every
      snapshot member acks succeeds.
- [ ] `VERSION_FORCED_INCOMPLETE` — `posted` against `complete(forced)` is refused **naming the
      participant**; after that participant acks, the same request succeeds.
- [ ] `STAGED_ENTITY_CHANGED` — an operator publish racing a head move is refused; the same publish
      with no concurrent move succeeds.
- [ ] `CATALOG_VERSION_UNKNOWN` — an unknown id is refused on **both** the resolve and diff paths;
      a known id succeeds on both. The single-door requirement is what the two-path form tests.
- [ ] `PARTICIPANT_UNKNOWN` — an ack from a principal outside the version's snapshotted set is
      refused **and audited**; an ack from a member succeeds.

**Negative control on a code that does not exist**

- [ ] No refusal path in this feature raises `FORCE_COMPLETE_QUORUM`. A ceremony refusal carries a
      `05-governance` gate code. The design set names this token only to deny it, and a
      grep for it in the crate must stay at zero.

## 7. Known unknowns

**The arithmetic of this section.** Fifty-one rows: **eighteen carried verbatim** from
[`../design/06-catalog-version.md`](../design/06-catalog-version.md) §6 — the slice's full count,
not a selection — and **thirty-three raised here**, across two review passes: eleven while
authoring, **twelve by the first three-lens pass** and **ten by the second**. Of the fifty-one,
**sixteen block no DoD in this document**: rows 3, 4, 5, 14, 34, 49, 50 and 51, plus the eight
resolved on **2026-08-31** — rows 22 and 41 by **P-D-52**, row 37 by **P-D-53**, row 30 by
**P-D-56**, and rows 1, 9, 10 and 11 by **P-D-60**, the first round over *carried* rows, answered in
`design/06` §6 first with the carry following. The other **thirty-five** each name the DoD they
block.

**A resolved row is kept in place rather than struck from the register**, because rows 41 and 45 cite
row 22 and a deleted record would break the citations. Of the four DoDs row 30 named,
`cpt-cf-bss-products-dod-request-door` is freed, while `dod-increment-request-port`, `dod-coalescer`
and `dod-posting-safe-observability` stay blocked by their own other rows; P-D-60 freed
`dod-composition-clear`, `dod-referential-delete-predicate`, `dod-request-queue` and
`dod-freeze-ledger-tables`.

Rows 14 and 34 block nothing for a reason that is itself the finding:
**no DoD in §5 declares the `validate(lint)` door**, because nothing in the design set specifies it,
and none names the archival or scale halves. Rows 49-51 block nothing because each asks what a
convention **means**, not what a door does.

**Carried, not answered.** A question is registered against **its owner's** register. Where the
owner is another document, the row carries a one-line pointer and nothing more; striking a
resolved record elsewhere can retract a decision's propagation, so none was touched.

### Carried verbatim from `design/06` §6

1. ~~**Does the composition-clear re-publish emit `SkuPublished` beside `SkuCompositionCleared`?**~~
   **Answered in the slice (owner call, 2026-08-31 — P-D-60): both, `SkuCompositionCleared`
   additive.** `design/06` §6 carries the answer and §2's `inst-cc-clear` the rule. Suppressing
   `SkuPublished` would leave the read model a version behind on the entity whose flag just changed,
   `08`'s projector keying on `publishedVersion` from `*Published`. Both carry the same entity and the
   same `publishedVersion`, so a consumer keyed on version sees one version change and **no
   obligation is created**; `12`'s additivity rule is not widened.
   Original text: `inst-cc-clear` routes the clear through 01's publish door, whose
   `inst-fd-publish-emit` fires
   `ProductPublished`/`SkuPublished` unconditionally, and 08's projector keys on `publishedVersion`
   from `*Published`. Neither slice says whether a consumer sees one event or two, and 12's
   additivity rule is scoped to 09's coalesced summary.
   **Blocks**: no DoD — **resolved by P-D-60**; `cpt-cf-bss-products-dod-composition-clear` is freed,
   while `dod-cv-events` stays blocked by rows 20, 27, 35, 39 and 47.
   **Owner**: was this feature with the events/audit consumer owner and `08-read-models`;
   **closed**.

2. **OPEN — which budget this slice carries.** `DESIGN.md` §1.2 reads "the < 3 s propagation and
   < 5 s posting-safe budgets on the slice-01 outbox + slice-06 freeze machine". Read distributively
   it gives the < 3 s budget to the outbox alone and this slice only `nfr-posting-safe-budget`; read
   jointly it splits both across both, which is how the sibling clause in the same sentence
   ("slice 06/10 storage posture") is claimed. The set has been written both ways in the last two
   days. **No slice §5 measures the < 3 s budget either way**, which is the owed probe and the
   thing that would settle it.
   **Blocks**: `cpt-cf-bss-products-dod-posting-safe-observability`.
   **Owner**: BSS Program Lead, with slices 01/06/08 — PRD §15's own routing.

3. **The v1 freeze participant, pricing, is §15-silent** (**P-D-48** narrowed the registered set to
   it; Contracts and Billing register at their own build time): the protocol ships registry-complete
   with `freeze_overdue` naming it from day one; until pricing's ack lands, every version is
   posting-unsafe by construction — correct, loud, and worth a product decision on v1 launch
   sequencing (the ack before first posted use).
   **Blocks**: no DoD. It is a launch-sequencing decision, and
   `cpt-cf-bss-products-dod-freeze-timeout` and `cpt-cf-bss-products-dod-liveness-and-release`
   both record the posture rather than depending on the answer.
   **Owner**: the product owner.

4. **Full-snapshot economics** (NFR #6): entry-per-entity manifests are O(catalog) per version; the
   §15/NFR-workshop publishes-per-day target bounds storage — the manifest table is designed for
   dedup later (the entity half references immutable version rows; the capture half stores copies
   — H3 — and is the part a delta-encoding would compress; a compatible optimization, named to keep
   it out of v1).
   **Blocks**: no DoD — deliberately out of v1.
   **Owner**: this feature, at the NFR workshop.

5. **Bulk-lane starvation**: a steady interactive trickle must not defer a bulk window past its
   5-min hard max — the coalescer's deadline logic gets a probe when built.
   **Blocks**: no DoD; the probe is already obliged by
   `cpt-cf-bss-products-dod-coalescer` and asserted in §6.
   **Owner**: this feature.

6. **`freezeComplete` = "all acked" regresses when a participant releases.** `inst-fz-ack` defines
   the predicate over the ledger's current value, and §4 makes `state` "four values, one column" —
   so the release door overwrites `acked` with `released` and the version flips back out of
   posting-safe. §4 already stores `acked_at` / `released_at`, so a timestamp-keyed predicate is
   available, but choosing it also moves slice 10's `version-liveness` pair.
   **Blocks**: `cpt-cf-bss-products-dod-ack-door`.
   **Owner**: this feature, with `10-retention-erasure`.

7. **Nothing creates the ledger rows, and an empty ledger satisfies "all acked".** The only stated
   creation point is consumption of `CatalogVersionPublished`, which is emitted after the increment
   transaction commits; `freeze_state` is a "derived cache of the ledger". In that window `posted`
   resolution of an entirely unfrozen version succeeds — the fail-closed default C5 and AC #21
   require, open by construction, and no `design/06` §5 probe looks at it. The operand exists: **P-D-49** gave
   slice 10's retention gate this version's `participant_set_snapshot` for the same vacuity, and
   `freezeComplete` can range over it too — but that is this feature's rule to change, not 10's.
   **Blocks**: `cpt-cf-bss-products-dod-ack-door`.
   **Owner**: this feature.

8. **`participant_set_snapshot` is stored twice and only one copy is inside the checksum.** §4 puts
   it on the `products_catalog_version` row; `inst-sn-collect` puts it in the capture store, whose
   bullet says "the checksum covers both halves". Which is authoritative — and therefore whether the
   participant set is inside the byte-identity checksum — is stated nowhere; `freeze_state` on the
   same row carries a "(derived cache)" annotation and this column does not. *Re-measured at
   `41d1baa5e`: the annotation asymmetry is exactly as stated.*
   **Blocks**: `cpt-cf-bss-products-dod-participant-set`,
   `cpt-cf-bss-products-dod-snapshot-builder`.
   **Owner**: this feature.

9. ~~**Is the capture store the same table as `products_catalog_version_entry`?**~~
   **Answered in the slice (owner call, 2026-08-31 — P-D-60): two tables.** The capture rows are
   **`products_catalog_version_capture`**, keyed `(tenant_id, catalog_version_id, capture_kind)`;
   the entity half keeps the name, the key and the index. One PK cannot express both keys, and a
   shared table would make every column of both halves nullable, admitting a row that is neither a
   valid entry nor a valid capture. **P-D-40 needs no re-aiming** — its predicate is written over
   `products_catalog_version_entry`, whose every row now references an entity version, so the
   predicate and the index are exactly right as written. This row's own owner clause was **inverted**
   on that point: two tables is the arm that owes no re-aiming, capture rows holding copies and
   referencing nothing per §4's H3 fix. Original text: One §4 bullet gives
   one table two disjoint keys and two disjoint column sets. This is not cosmetic: 01 **P-D-40**'s
   DELETE predicate is written over that table name, so on the one-table reading the guard's
   subquery also scans capture rows that reference no entity version, and the index at §4 was added
   for the entity half only.
   **Blocks**: no DoD — **resolved by P-D-60**;
   `cpt-cf-bss-products-dod-referential-delete-predicate` — **the flagship** — is freed, and
   `dod-version-entry-table` stays blocked by row 49.
   **Owner**: was this feature with whoever re-aims P-D-40; **closed**, and no re-aiming is owed.

10. ~~**Who writes the request states `superseded` and `coalesced`, and `satisfied_by_version_id`,
    and what leaves them?**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-60): the increment transaction writes two,
    and `superseded` is struck.** The transaction that allocates the id, builds the manifest and
    emits `CatalogVersionPublished` marks each request it satisfied `coalesced` and stamps its
    `satisfied_by_version_id` — it is the transaction that produces the `satisfiedRequests` set, the
    set **P-D-50** gave the column its existence to let a replay rebuild. `coalesced` is
    **terminal**, which answers *"what leaves them"*. Nothing supersedes a request, so the roster
    becomes `(pending, coalesced)`.
    **A carry-fidelity note**: `design/06` §6 asks this of **`superseded` alone**; this row widened it
    to all three and added the P-D-50 sentence. The widening is correct on the measurement — none of
    the three had a writer — but it is a departure from verbatim this section's preamble does not
    declare among its three. Original text: No instruction in `design/06` §2 or §3 writes any of the
    three, and
    `satisfied_by_version_id` is the **P-D-50** column whose stated purpose is that without it a
    replayed `CatalogVersionPublished` cannot have its `satisfiedRequests` set rebuilt and pricing's
    stuck pending refs cannot be reconciled. On `superseded` specifically: `inst-sn-revalidate` says a failed mechanical run "re-coalesces and retries
    fresh, the request never lost", which the PRD echoes as "A request is never dropped". The value
    is either dead or an unwritten obligation.
    **Blocks**: no DoD — **resolved by P-D-60**; `cpt-cf-bss-products-dod-request-queue` is freed.
    **Owner**: was this feature; **closed** — the door named for two values, the third struck.

11. ~~**What is `products_freeze_ack.state`'s transition table?**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-60): six edges, and one of the three
    sub-questions was already answered elsewhere in this document.**
    `cpt-cf-bss-products-dod-force-completion` already stated that a recovered participant's later ack
    moves the row to `acked` and *"the stale stamp frees nothing"* — so `released_at` is **not**
    cleared and is write-once. The other two follow from the doors' own wording: force-completion
    records *each **missing** participant*, so it never overwrites `acked` or `released`; and the
    release door's precondition is holding no live references rather than having acked, so
    **`pending → released` is admitted**. `released` is terminal, no other transition is admitted, and
    **the table has no entry point** — who writes `pending` is row 46's and what creates the ledger
    rows is `design/06` §6's, both open. `freezeComplete`'s formula regression on `acked → released`
    stays row 6's. Original text: Unstated: whether `pending` may go
    straight to `released`, whether force-completion may overwrite a row already `acked` or
    `released`, and whether a forced participant's later ack clears the `released_at` the ceremony
    stamped. Each answer changes both `freezeComplete` and slice 10's collection gate, which reads
    the pair.
    **Blocks**: no DoD — **resolved by P-D-60**; `cpt-cf-bss-products-dod-freeze-ledger-tables` is
    freed, while `dod-force-completion` stays blocked by rows 26, 31 and 33 and
    `dod-liveness-and-release` by rows 31, 33 and 46. **§4 of this document now states the six edges**
    rather than declining a machine, because the edges are the slice's answer and not this document's
    invention.
    **Owner**: was this feature with `10-retention-erasure`; **closed** for the edges.

12. **What is the resolution API's transport and route?** `IntentfulResolver` is the only door in
    this slice with no route: the increment door and the diff both carry one, 08 explicitly puts the
    surface out of its scope, and 01 hands this slice the intent clause without a surface. 12's
    qualifier grammar means this slice cannot simply add the authoring-publish contract id while 01
    claims it unqualified.
    **Blocks**: `cpt-cf-bss-products-dod-intentful-resolver`.
    **Owner**: this feature, with `01-foundation`.

13. **What door consumes `catalog_version × publish`?** 05's RBAC roster grants six actions on this
    slice's resource; this slice names doors for five, and its operator lane goes through the request
    door instead ("an entity publish NEVER enqueues an increment"). Either the roster grants an
    action no route consumes, or this slice is missing a door. *Re-measured at `41d1baa5e`: the
    roster does grant six — `request`, `ack`, `release`, `read`, `force_complete`, `publish`. A bare
    grep for the full token `catalog_version × <action>` returns only **five**, because 05 §3.2
    spells the ack/release row as `` `catalog_version × ack`, `× release` `` with the prefix elided.
    The count is six; do not correct it down.*
    **Blocks**: `cpt-cf-bss-products-dod-cv-authz`.
    **Owner**: this feature, with 05's roster owner.

14. **Which slice builds the `validate(lint)` door?** §6 claims `fr-prepublish-lint` and AC #45 —
    the claim is now matched by a §1.5 scope line — but no instruction, store, RBAC pair, error code
    or probe in this slice delivers the structured per-entity report `PRD` §6.13 requires, and 09
    consumes a report from a producer that does not exist.
    **Blocks**: no DoD in §5 — **and that absence is the finding.** `fr-prepublish-lint` appears in
    §1.2's requirement roster because DECOMPOSITION §2.6 assigns it, and this document
    deliberately declares no DoD for it rather than inventing a door the design set does not
    specify.
    **Owner**: the design-set owner, with this feature and `09-bulk-promotion`.

15. **The manifest's row collections have no named sort key.** `inst-sn-checksum` rested on
    **P-D-28**, which states in terms that it orders fields and *not* rows; **P-D-29** supplies a row
    rule but scopes it to the category-assignment and attribute-value sets "inside the content". The
    manifest's entry rows and capture rows are neither, so two runs or two engines may hash the same
    snapshot differently — against C4, AC #20 and `design/06` §5's byte-identity flagship. *Re-measured at
    `41d1baa5e`, and part of it is settled by shipped code: `domain::canonical`'s own doc fixes
    **where** the sort is paid — *"The first door whose payload carries a collection owes that sort
    **here**, rather than at its own call site"* — names the nested complete-set roster as owed with
    it, **and states the key for the two sets P-D-29 does name**: *"by the collection's own
    identifier"*. What is open is therefore narrower than "no named key": it is whether that rule
    reaches the manifest's **entry** and **capture** rows, which are neither of P-D-29's two sets
    and carry no collection identifier of the same kind.*
    **Blocks**: `cpt-cf-bss-products-dod-snapshot-builder`.
    **Owner**: whoever owns 01 §4.3's canonicalization pin (P-D-29), with this feature.

16. **`staged_at` has no admitted writer.** A `staged_at` column implies the version row exists
    before publication, while the only stated insert is inside the commit transaction and
    `freeze_state`'s roster has no staged value for such a row to occupy — and an insert at stage
    would burn an id on every `STAGED_ENTITY_CHANGED` refusal, against the gapless guarantee C1 and
    `inst-cvc-serial` both assert.
    **Blocks**: `cpt-cf-bss-products-dod-version-counter`,
    `cpt-cf-bss-products-dod-catalog-version-table`.
    **Owner**: this feature.

17. **The `commit → durable-acceptance` meter is declared by no slice.** `design/06` §3.3
    decomposes NFR #4's
    program SLO into three meters and attributes this one to 01; 01 declares no observability
    surface and records its NFR #3 probe as owed, while 08 also names the meter as 01's. The
    posting-safe composite is declared derivable from three meters when one is declared nowhere.
    *Re-measured at `41d1baa5e`: `design/01` carries no observability section; its only mention is
    `inst-fd-rule-registry`'s "`rule_names()` for observability only".*
    **Blocks**: `cpt-cf-bss-products-dod-posting-safe-observability`.
    **Owner**: this feature, with `01-foundation` and `08-read-models`.

18. **`freezeComplete` and `freeze_state` are one concept with two names and two shapes.** `PRD` §3
    defines `freezeComplete` as "A per-`CatalogVersion` **flag**" and §6.6 makes it a **MUST expose**
    obligation per `catalogVersionId`; §4 of this slice stores `freeze_state ∈ {open, complete,
    complete(forced)}`; and **P-D-19** writes `freezeComplete = complete(forced)`, which is coherent
    only under the state reading. The pass made this slice internally consistent by keeping
    `freezeComplete` as `inst-fz-ack`'s predicate and pointing the state assignments at the §4
    column — which surfaced the divergence rather than settling it. Owed: whether the exposed flag
    is derived from the column (and what the resolution API returns at `complete(forced)`), and
    whether P-D-19's phrasing is amended.
    **Blocks**: `cpt-cf-bss-products-dod-intentful-resolver`,
    `cpt-cf-bss-products-dod-ack-door`.
    **Owner**: this feature, with the PRD owner.

### Raised here rather than carried

Four came from the counterpart gear's shipped port, one from that gear's dev registry, one from
`gears/bss/libs/coord`, and five from this gear's own crate. Every quotation below was byte-verified
against source at `41d1baa5e`.

19. **The shipped port carries no `source`, `lane` or `operation_key`.**
    `CatalogVersionRegistryV1::request_version` takes `request_id: &str` and nothing else, while
    this feature's `IncrementRequest` is `(source, lane ∈ {interactive, bulk}, request_key,
    operation_key?, requested_at)` and its uniqueness is on `(tenant_id, source, request_key)`. So **D-47's
    two-lane split and the `operation_key` coalescing have no operand on the only shipped caller**:
    pricing, the v1 registered set's one member, can express one lane. Whether the adapter supplies
    defaults, or the port widens, is not this document's to decide.
    **Blocks**: `cpt-cf-bss-products-dod-increment-request-port`.
    **Owner**: this feature, with pricing's SDK owner.

20. **`pending_ref` is issued by the registry and never echoed back.**
    `PendingVersionRef { request_id, pending_ref }` gives the caller a registry-minted handle that
    `PlanPublished` carries, while `CatalogVersionPublished` carries `satisfiedRequests` keyed on
    `(source, request_key)`. A pricing row keyed on `pending_ref` alone therefore cannot be closed
    from the event. Whether `satisfiedRequests` should carry the pending ref too, or the adapter is
    obliged to keep the mapping, is open.
    **Blocks**: `cpt-cf-bss-products-dod-increment-request-port`,
    `cpt-cf-bss-products-dod-cv-events`.
    **Owner**: this feature, with pricing's SDK owner.

21. **`committed_version` is a poll and this feature declares no door for it.** The shipped port's
    second method resolves a pending ref to its committed version, and pricing has **exactly one**
    caller of it — the `ReadModelWarmJob` sweep, which `infra::registry_deadline`'s own doc records
    as *"awaited once, from the read-model warm sweep"*. `module.rs`'s *"One requester, two readers"*
    counts **holders of the registry**, not callers of this method. This feature's doors are request, read, ack,
    release and force_complete, and its resolver is keyed on `catalogVersionId`. **Implementing the
    port today leaves one of its two methods with no surface.**
    **Blocks**: `cpt-cf-bss-products-dod-increment-request-port`,
    `cpt-cf-bss-products-dod-intentful-resolver`.
    **Owner**: this feature, with pricing's SDK owner.

22. ~~**The shipped port has a fourth error arm this feature's roster cannot produce.**~~
    **Answered (owner call, 2026-08-31 — P-D-52): the request door owes the code.**
    `REQUEST_SOURCE_UNKNOWN` is minted, declared in `design/06` §3.2 and raised by
    `inst-cv-request` alone. Its shape is not a free choice: the port reaches `Rejected` only
    on a `FailedPrecondition` **carrying a precondition violation of type
    `CATALOG_VERSION_REJECTED`**, so the refusal is 422-architectural, 400 on the wire, and
    **not** the 403 an analogous roster miss would take. The body below is kept because rows
    41 and 45 cite this row's owner pairing. Original text: Beside
    Unconfigured / Unreachable / Internal it carries **`Rejected`**, discriminated by the wire
    constant `CATALOG_VERSION_REJECTED`, and argues that *"a refusal is a decision and will be made
    identically for as long as the request is unchanged; an outage is a deployment state a retry may
    find changed."* None of this feature's six codes **was** a refusal **of an increment request** — the
    door refuses only an unregistered source, and §3.2 declares no code for it. So either the
    request door owes a refusal code, or the port's `Rejected` arm is unreachable against this
    registry.
    **Blocks**: no DoD — **resolved by P-D-52**; `cpt-cf-bss-products-dod-request-door` and
    `cpt-cf-bss-products-dod-cv-error-taxonomy` both carry the answer.
    **Owner**: was this feature with pricing's SDK owner; **closed**.

23. **Pricing's dev-minted version space collides with this feature's counter, and the sweep has no
    owner.** `LocalDevCatalogVersionRegistryV1` mints from `Utc::now().timestamp_millis()` — order
    10¹² — while this feature's counter is expected to start low. `CatalogVersion` is `Ord` and
    pricing's pin-eligibility frontier is **prefix-closed**, so every version this registry issues
    sorts earlier than every dev-minted one and the frontier cannot advance past the contamination.
    The dev module names the sweep (`LIKE 'dev-local-%'` over
    `pricing_plan_revision.pending_version_ref`) and its own doc says *"Nothing here is safe to run
    beside a real registry, and nothing here should outlive one"* — and assigns the sweep to nobody.
    **The ordering half of this row rests on a premise no document states**: nothing in the PRD, in
    `design/06` §4 or in this document's own
    `cpt-cf-bss-products-dod-version-counter` pins the counter's **initial value**, and the
    conclusion holds only for a start far below 10¹². Whether that value is pinned, and to what, is
    part of what this row asks.
    `design/06`'s *"their adoption is one event handler"* is **not** the claim this row
    contradicts — that clause sits inside `inst-cc-clear` and is about pricing registering the
    inbound composition signal, which the version space does not touch.
    **Blocks**: `cpt-cf-bss-products-dod-version-counter`,
    `cpt-cf-bss-products-dod-increment-request-port`.
    **Owner**: this feature, jointly with pricing.

24. **The manifest checksum has no `digest_version` companion.** `design/06` §4 gives
    `products_catalog_version` a `checksum` and no digest-version column, while
    `products_entity_version` carries one — and `domain::canonical::DIGEST_VERSION`'s own doc gives
    the reason in terms that apply identically to a manifest: *"Storing it on the row is what lets
    slice 10's restore drill re-verify a sampled entity version against the rule it was actually
    computed under; without it, version-history corruption is invisible to every checksum."* Same
    drill, same argument, one column short.
    **Blocks**: `cpt-cf-bss-products-dod-catalog-version-table`,
    `cpt-cf-bss-products-dod-snapshot-builder`.
    **Owner**: this feature, with `10-retention-erasure`.

25. **An unpaid cross-engine truncation stands under this feature's byte-identity flagship.**
    `canonical::render_instant` truncates to microseconds, and its own doc states the residual
    hazard: `Utc::now()` carries nanoseconds, SQLite stores nine digits, Postgres `timestamptz`
    **rounds** to six, so `...:00.123456789Z` renders `.123456` on one engine and `.123457` on the
    other and *"the same logical entity is frozen under two different `content` strings and two
    different `content_digest` values on the two engines."* The fix is named — truncate the instant
    **where it is written**, at the head-row insert — and is recorded **only in code doc comments**,
    in no plan and no artifact.
    **Blocks**: `cpt-cf-bss-products-dod-snapshot-builder`, and the second open clause of
    `cpt-cf-bss-products-dod-version-history-table` (the golden vector *asserted byte-identical
    across engines*), which is why **this feature does not tick that DoD**.
    **Owner**: `01-foundation` — the create doors are where the write is. One-line pointer only.

26. **`GovernanceGate`'s subject cannot name a catalog version or a participant set.**
    `GovernanceGate::evaluate` takes an `EntityRef` whose `entity_kind` is
    `bss_products_sdk::models::EntityKind`, and that enum is exactly `Product | Sku`. So
    `catalog_version × force_complete` and `freeze_participant × write` — both governed acts of this
    feature — have no expressible subject. This is a **wrong-typed subject**, not a missing operand,
    and it is the same class as `features/governance.md`'s own record that `PreAuthorized` cannot
    admit a cascade leg or a bulk row.
    **Blocks**: `cpt-cf-bss-products-dod-force-completion`,
    `cpt-cf-bss-products-dod-participant-set`.
    **Owner**: `05-governance`, with this feature.

27. **Three of this feature's four events need a body with no entity dimension.** `EventBodyCore` is
    `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`, and its module doc rules
    out putting **`publishedVersion`** on it as a sixth field, on the ground that
    `ProductDiscarded` *"writes no version at all"* and would have to invent a value. That argument
    is specific to that field and does not close the core to extension in general — which is part of
    what makes the shape below a decision rather than a deduction. A catalog version and a participant set have none of the five, so
    `CatalogVersionPublished`, `FreezeForceCompleted` and `FreezeParticipantSetChanged` need a
    second core rather than an extension of the first. `SkuCompositionCleared` fits the existing
    shape. What the new core's field set is, and whether §4.5's "one body core" sentence is amended
    or scoped to Foundation events, is open.
    **Blocks**: `cpt-cf-bss-products-dod-cv-events`.
    **Owner**: this feature, with the events/audit owner and the PRD §4.5 owner.

28. **This feature adds the SDK's first write method, and the trait's shape is undecided.**
    `bss_products_sdk::api::ProductsClient` ships `get_product` and `get_sku` and calls itself *"the
    in-process contract for **reading** registry entities"*. Whether the increment contract widens
    that trait — changing what every existing implementor must provide — or arrives as a second
    trait beside it, is a contract decision this document declines to take.
    **Blocks**: `cpt-cf-bss-products-dod-increment-request-port`.
    **Owner**: this feature, with `12-consumer-contracts`, which owns the SDK type's audience.

29. **What is the cardinality cost of a per-tenant increment lease?** `gears/bss/libs/coord` is the
    shared primitive, and a per-tenant key is **precedented**: `bss-ledger` keys
    `recognition-run:{tenant}:{period_id}` and
    `period-close:{tenant_id}:{legal_entity_id}:{period_id}`, and the README offers one per
    `(tenant, period)` as a typical fit. *(An earlier draft of this row called the shape "BSS's
    first per-tenant one", generalising a doc comment that speaks for one of pricing's three sweep
    keys. Withdrawn — the precedent question is settled and only the cost is open.)* A per-tenant key
    puts a `coord_leases` row per active tenant in the increment path; whether that is right, or the
    worker should shard tenants under a bounded key set, is open — and it is the one row here whose
    answer changes an SLO rather than a schema.
    **Blocks**: `cpt-cf-bss-products-dod-coalescer`.
    **Owner**: this feature, with the `bss-coord` owner.


30. ~~**Is the door's answer time this feature's to publish, and is five seconds it?**~~
    **Answered (owner call, 2026-08-31 — P-D-56): yes to publish, no to five, and there are two
    budgets rather than one.** The **acknowledgement** budget is a shape: the door stamps
    `requested_at`, claims idempotently, enqueues and answers, taking no lease and making no
    cross-gear call, so it fits inside the *smallest* value a consumer may configure — its
    `registry_call_timeout_secs` rejects `0` and caps at 60 — rather than inside that default of
    five. The **batching SLO** is already published and is C1's, `requested_at → published_at`
    p95 ≤ 60 s / max 5 min, which is the referent the shipped consumer's *"batching SLO"* means; the
    ≤ 5 s window and the five-minute bulk maximum are inputs to it, not the door's answer time.
    **The conditional clause below is settled too: the door does not take the lease** — `design/06`
    §2 rule 2 gives it to the coalescer that drains the queue, and this document's own contradicting
    sentence was corrected in the same round. Original text: Pricing holds
    every cross-gear registry call to `DEFAULT_REGISTRY_CALL_TIMEOUT_SECS = 5`, and
    `infra::registry_deadline`'s doc gives the reason: *"a hung peer pins a transaction, its row
    locks and a pool connection on every mutating path at once"*, with ten of twelve call sites
    inside an open write transaction. No document in this gear's design set states a bound on the
    increment door's own answer time; C1 bounds the **version**, not the call. So the budget the
    door is actually held to is declared only in the caller's crate.
    **Blocks**: no DoD — **resolved by P-D-56**; `cpt-cf-bss-products-dod-request-door`,
    `cpt-cf-bss-products-dod-increment-request-port`, `cpt-cf-bss-products-dod-coalescer` and
    `cpt-cf-bss-products-dod-posting-safe-observability` all carry the answer.
    **Owner**: was this feature with pricing's SDK owner; **closed** — nothing in the consumer's
    crate or register is changed by it.

31. **Four more of this feature's doors have no route, and the design set records them as
    routeless rather than as absent.** `05-governance` §3.2 marks `catalog_version × ack`,
    `× release`, `× force_complete` and `freeze_participant × write` *"no route declared"*, and its
    own §6 counts eleven such rows gear-wide. Without a surface, a participant has nothing to send
    an ack or a release to, so the whole `VERSION_FORCED_INCOMPLETE` lifting path is unbuildable and
    §6's ack-identity criterion is untestable. Whether the fix is declaring the routes or admitting
    the grants are unspent is not this document's call.
    **Blocks**: `cpt-cf-bss-products-dod-ack-door`,
    `cpt-cf-bss-products-dod-liveness-and-release`,
    `cpt-cf-bss-products-dod-force-completion`, `cpt-cf-bss-products-dod-participant-set`.
    **Owner**: this feature, with 05's roster owner.

32. **Which lane does the `requested_at → published_at` maximum of five minutes belong to?** A bulk
    batch whose window closes at the five-minute hard max cannot also publish inside a five-minute
    maximum measured from the same instant, so the SLO is unsatisfiable for every bulk batch that
    runs to its bound. `design/06` C1 and `inst-cv-slo` state both numbers in one clause without
    scoping either, and §6 treats them as separate without saying which meter carries the maximum.
    **Blocks**: `cpt-cf-bss-products-dod-posting-safe-observability`,
    `cpt-cf-bss-products-dod-coalescer`.
    **Owner**: the owner of pricing D-47 / C1, with this feature.

33. **Which acts write `products_freeze_ack.released_at`?** `design/06` §4 gives the column its own
    existence so the release fact *"cannot be read as an ack or as a release through the
    participant's own door"*; the force-completion ceremony stamps it with `state` unchanged; and
    the retention gate reads the `(state, released_at)` pair. **No location says whether the
    participant's own release door stamps it too.** Row 11 asks for the state transition table and
    does not reach the column's writers.
    **Blocks**: `cpt-cf-bss-products-dod-liveness-and-release`,
    `cpt-cf-bss-products-dod-force-completion`.
    **Owner**: `10-retention-erasure`, jointly with this feature — its gate reads the pair.

34. **Do the manifest DoDs discharge `nfr-snapshot-archival-dr` and `nfr-scale-extensibility`, or is
    a further DoD owed?** Both ids are in §1.2's roster and both are divided at §1.2's table, yet no
    DoD in §5 and no criterion in §6 names archival, durability, restore, scale or capacity. Row 4
    records the scale half as deliberately out of v1; the archival half is recorded nowhere. So the
    arithmetic above, which presents row 14 as the single requirement with no deliverer, may
    understate by one. The FEATURE template gives a DoD no requirement field, so nothing mechanical
    settles it.
    **Blocks**: no DoD — it asks whether one is missing.
    **Owner**: the design-set owner, with `10-retention-erasure`.

35. **Does "exactly four broker events" bind this feature's emissions or a consumer's observed
    count?** `cpt-cf-bss-products-dod-cv-events` states the four as an absolute MUST while row 1
    records that the composition clear may emit `SkuPublished` beside `SkuCompositionCleared`,
    because it runs through 01's publish door. `design/06` §4 lists the four with the same silence.
    **Blocks**: `cpt-cf-bss-products-dod-cv-events`.
    **Owner**: the events/audit consumer owner, with `08-read-models` — the same pair row 1 names.

36. **What is the freeze timeout's config field, its owner and its unit, so the export has a
    destination?** `cpt-cf-bss-products-dod-freeze-timeout` obliges
    `resolved_idempotency_retention_hours`'s clamp to take `max_freeze_timeout` as its lower bound.
    `design/01-foundation.md` says only that slice 06 exports the number and the store reads it as
    config; `design/06` says only *"config (coalescing windows, freeze timeout…)"*. No document
    names a field, a type, a unit, or what the **maximum** ranges over — tenants, lanes, or
    configured values. `ProductsConfig` carries hours as `u32`.
    **Blocks**: `cpt-cf-bss-products-dod-freeze-timeout`.
    **Owner**: this gear's config owner, with `01-foundation`'s.

37. ~~**At what isolation level does the increment transaction run?**~~
    **Answered (owner call, 2026-08-31 — P-D-53): the engine default, `READ COMMITTED` on
    Postgres.** The race is closed by `inst-sn-revalidate`'s row-version guard, never by isolation:
    of the three levels, only the default lets the guard fire and produce §6's required refusal — a
    snapshot-isolating level returns the collect-time snapshot so it cannot fire, and `SERIALIZABLE`
    aborts instead of raising the code. The word *"serialized"* in `inst-sn-collect` is the
    coalescer's one-worker-per-tenant serialization, not a database level. P-D-53 also scopes itself:
    it does not settle `03-sku-classification`'s removal-vs-publish race, which has no row to
    version. Original text:
    **At what isolation level does the increment transaction run?** The snapshot is collected
    *"inside the serialized transaction"* and the heads are re-read *"before commit"* in the same
    transaction. Under a snapshot-isolating level that re-read returns the collect-time snapshot and
    the race is undetectable; under read-committed it is detectable; under serializable the
    transaction aborts rather than raising `STAGED_ENTITY_CHANGED`, which §6 requires as a refusal.
    No isolation level is stated anywhere in the design set or the crate.
    **Blocks**: no DoD — **resolved by P-D-53**; both
    `cpt-cf-bss-products-dod-stage-commit-revalidation` and
    `cpt-cf-bss-products-dod-snapshot-builder` carry the answer.
    **Owner**: was this feature with the storage-posture owner; **closed**.

38. **Which act refreshes `freeze_state` to `complete`?** `design/06` §4 annotates the column *"(derived
    cache of the ledger)"* and this document forbids it being the authority; the force-completion
    ceremony writes `complete(forced)`; and no DoD writes `complete`. Whether the last ack writes
    the cache in the same transaction, or it is recomputed on read, and what the PRD's per-version
    **MUST expose** obligation returns while the cache reads `open` on an all-acked version, is
    unstated. Rows 6, 7 and 18 concern the predicate and the naming, not the cache's refresh.
    **Blocks**: `cpt-cf-bss-products-dod-ack-door`,
    `cpt-cf-bss-products-dod-catalog-version-table`.
    **Owner**: this feature, with the PRD owner — the pairing row 18 already names.

39. **When four non-Foundation events join the roster, does `design/01` §4.5's list become the
    gear's list?** `events_tests`' own doc says the eight names are *"transcribed from the design's
    own sentence"*, and that section is titled **Foundation-owned events**. Extending the constant
    changes what it transcribes from one section's sentence to the union of two documents, which is
    the property that doc says makes the test more than a tautology. Row 27 asks only whether the
    "one body core" sentence is amended.
    **Blocks**: `cpt-cf-bss-products-dod-cv-events`.
    **Owner**: the events/audit owner, with `01-foundation`'s.

40. **Does a slice's §2 instruction step or its §4 storage shape govern a column-level fact?** This
    document states both that the slice's instruction steps *"stay normative"* and that
    `design/06` §4 *"governs on any column-level fact"*, and then applies the second against a §2
    step — `inst-fz-ack`'s `(version, participant)` — in
    `cpt-cf-bss-products-dod-ack-door`. The sibling FEATUREs state the precedence only over §5
    versus §4, never over §2 versus §4. The answer decides whether `inst-fz-ack` needs editing or is
    correct as a shorthand.
    **Blocks**: `cpt-cf-bss-products-dod-ack-door`,
    `cpt-cf-bss-products-dod-increment-request-port`.
    **Owner**: the design-set owner.

41. ~~**Does the request door owe a refusal code, or is the error algo's Input clause scoped?**~~
    **Answered with row 22 (owner call, 2026-08-31 — P-D-52): a seventh code is owed, and the
    Input clause stands unnarrowed.** `REQUEST_SOURCE_UNKNOWN` is that code. Original text:
    `cpt-cf-bss-products-algo-catalog-version-errors` declares its Input to be *"a refusal raised by
    any door of this feature"*, and §2 documents one refusal with no code: *"A request whose `source`
    is not a registered requester is refused at the door."* Row 22 records the same gap from the
    counterpart port's side. Either a seventh code is owed or the Input clause is narrowed; minting
    a code here would author the taxonomy.
    **Blocks**: no DoD — **resolved by P-D-52** with row 22.
    **Owner**: was this feature with pricing's SDK owner; **closed**.


42. **What is "the manifest header"?** `design/06` §4 lists it as the last item of
    `products_catalog_version`'s column set and no document in the tree states its field set, its
    type, or whether the checksum covers it. Every other column on that table has a stated shape or
    a row here — `staged_at` at row 16, `participant_set_snapshot` at row 8, the missing
    `digest_version` at row 24 — and this one has neither. Until it is decided the row cannot be
    created.
    **Blocks**: `cpt-cf-bss-products-dod-catalog-version-table`,
    `cpt-cf-bss-products-dod-snapshot-builder`.
    **Owner**: this feature, with whoever owns `design/06` §4.

43. **Under which absence mode, and against which roster, is the manifest rendered?**
    `domain::canonical`'s entry point takes the mode as a **required** argument precisely because the
    wrong choice is undetectable — *"a caller that picked the wrong one would produce a plausible
    string and a wrong digest, and nothing downstream could tell"* — and the complete-set arm
    additionally requires a **declared roster**, since *"a set is only complete against a declared
    roster, so the roster travels with the mode rather than being inferred from the value"*. No
    document says which arm the manifest takes, or what its roster is. This is the same pairing row
    15 names and belongs beside it.
    **Blocks**: `cpt-cf-bss-products-dod-snapshot-builder`.
    **Owner**: whoever owns 01 §4.3's canonicalization pin, with this feature.

44. **Is `09-bulk-promotion`'s export door a third consumer of the shared version lookup, and who
    refuses an unknown id there?** `cpt-cf-bss-products-dod-intentful-resolver` obliges one component
    to be *"the single raising door of `CATALOG_VERSION_UNKNOWN` for both resolve and diff"*, while
    `05-governance` §3.2 records a third route taking a `catalogVersionId` and spending the same
    `× read` grant — 09's export door. Either that door resolves through this feature's component,
    keeping the single-door clause true, or it raises its own refusal and the clause is false as
    written.
    **Blocks**: `cpt-cf-bss-products-dod-intentful-resolver`, `cpt-cf-bss-products-dod-cv-authz`.
    **Owner**: this feature, with `09-bulk-promotion`'s owner and 05's roster owner.

45. **What bounds `max_freeze_timeout` against the retention ceiling?**
    `cpt-cf-bss-products-dod-freeze-timeout` obliges the export into
    `resolved_idempotency_retention_hours`'s clamp, and the shipped call is **two-sided**: its upper
    bound exists so *"the resolution stays total"*. `u32::clamp` requires `min <= max`, so a
    `max_freeze_timeout` above the ten-year ceiling turns every expiry stamp into a panic. Row 36
    asks for the field, its owner and its unit and does not reach this interaction.
    **Blocks**: `cpt-cf-bss-products-dod-freeze-timeout`.
    **Owner**: this gear's config owner with `01-foundation`'s — the pairing row 36 names.

46. **Who writes `products_freeze_ack.state = pending`, and is the value live or dead?** No DoD in
    §5 names an act that writes it: the ack door writes `acked`, force-completion writes
    `not_frozen(forced)`, the release door writes `released`. Yet
    `cpt-cf-bss-products-dod-liveness-and-release` rests its whole v1 posture on *"a `pending`
    registration holds the version"*. This document applies exactly this test to the request
    queue's values at row 10 and to `staged_at` at row 16, and not here. Row 11 asks for the
    transition table and row 33 for `released_at`'s writers; neither asks whether `pending` has a
    writer at all. Deciding it would author the ledger's creation point, which is what §4 of this
    document declines.
    **Blocks**: `cpt-cf-bss-products-dod-ack-door`,
    `cpt-cf-bss-products-dod-liveness-and-release`.
    **Owner**: this feature, with `10-retention-erasure` — its gate reads the pair.

47. **Do the `SUBJECT_TYPE` ids P-D-51 pins extend to a subject that is not an entity, and where do
    causation and actor land for a body that does not exist yet?** **P-D-51** is not in §1.4's
    decision roster, which stops at P-D-50, and both its arms bear on this feature: arm 1 moves
    causation and the pseudonymous actor onto the **payload** because the transport has no slot for
    them, and arm 2 pins `subject_type` as the Product and SKU namespaces — neither of which a
    catalog version or a participant set is. Row 27 asks for the body core and does not reach
    either arm.
    **Blocks**: `cpt-cf-bss-products-dod-cv-events`.
    **Owner**: the events/audit owner with the PRD §4.5 owner — the pair row 27 names.

48. **May a per-action `Doors` cell in `05-governance` §3.2 carry several routes?** **P-D-50** fixed
    that *"a cell is per action"*, and `× read`'s cell names 09's export door. This feature spends
    `× read` at two further doors — the diff, which is routed, and the resolver, which is not — so
    either the cell's grammar admits several routes, or lint 3's route population is short, or rows
    12, 13 and 31 should have counted `× read` as routed-but-under-declared. This feature cannot
    widen another slice's table grammar.
    **Blocks**: `cpt-cf-bss-products-dod-cv-authz`, `cpt-cf-bss-products-dod-diff-door`.
    **Owner**: 05's roster owner, with the P-D-45/P-D-50 owner.

49. **Does "column-level fact" reach a row population?** Row 40 asks whether a slice's §2
    instruction step or its §4 storage shape governs a **column-level fact**, and applies it to a
    key. The capture-store roster is a different shape of the same question: §4's bullet carries
    **seven** `capture_kind` values and `inst-sn-collect` and `inst-df-diff` each list **six**,
    omitting `category values`. This document takes §4 as governing and says so at the site, but the
    precedence for a set of admitted row values rather than a column is stated nowhere. Either
    answer edits a document this feature does not own.
    **Blocks**: `cpt-cf-bss-products-dod-snapshot-builder`,
    `cpt-cf-bss-products-dod-diff-door`, `cpt-cf-bss-products-dod-version-entry-table`.
    **Owner**: the design-set owner — row 40's owner.

50. **Is §6 owed one criterion per DoD, or is it a deliberately selected set?** §6 states its own
    completeness only for the positive controls — *"seven codes, seven lines"* — while several DoDs have
    no criterion, among them `cpt-cf-bss-products-dod-cv-authz`, whose body argues the opposite
    discipline (*"the DoD names them as lines because a blanket criterion is ticked by
    inspection"*), `cpt-cf-bss-products-dod-cv-events`,
    `cpt-cf-bss-products-dod-increment-request-port`, `cpt-cf-bss-products-dod-require-broker` and
    the export clamp of `cpt-cf-bss-products-dod-freeze-timeout`. The FEATURE template gives a DoD
    no requirement field, so nothing mechanical settles it — the same absence row 34 notes for a
    different purpose.
    **Blocks**: no DoD directly; it decides whether §6 is short.
    **Owner**: the design-set owner.

51. **Does `pN` bind a delivery wave or a per-id importance?** `cpt-cf-bss-products-dod-cv-events`
    and `cpt-cf-bss-products-dod-cv-audit` are `p1` and both oblige work whose flow and DoD are
    `p2` — `cpt-cf-bss-products-flow-composition-clear` and
    `cpt-cf-bss-products-dod-composition-clear`. Neither this document, the FEATURE template nor
    `docs/checklists/FEATURE.md` states what the marker binds; the checklist constrains only that
    `featstatus` be consistent with the flow, algo, state and dod checkbox states, not their
    priorities. On the wave reading, one of these markers must move; on the importance reading,
    nothing is wrong.
    **Blocks**: no DoD; it decides whether four markers are misassigned.
    **Owner**: the design-set owner.

### Owed to other documents, recorded and deliberately not edited

Each is a one-line pointer into its owner's register. None was edited here.

- **`features/foundation.md` §1.4** claims `artifacts.toml` excludes design slices from
  autodetection. It is false — `[systems.autodetect.artifacts.DESIGN_SLICE]` carries
  `pattern = "design/*.md"` and `traceability = "FULL"`. That document is the shape donor for the
  six unwritten FEATUREs, so the cost compounds.
- **`infra/storage/entity/entity_version.rs`**'s module doc says *"no repository function reads or
  writes this table yet"*; `repo::insert_entity_version` exists and has two production callers.
  Owner: `01-foundation`'s code.
- **`features/governance.md` open item 14** records that the auto-satisfied `system_signal`'s
  *"signal reference as the authorizing principal" has no column*, the decision key being
  `(approval_id, approver_principal)`. `cpt-cf-bss-products-dod-composition-clear` obliges that
  write, so the approver of every auto-satisfied clear is unrecorded until 05 answers.
  Owner: `05-governance`.
- **`design/06` `inst-sn-collect` and `inst-df-diff`** each list six capture kinds where that
  slice's own §4 carries seven; the omitted kind is `category values`. Owner: that slice — row 49
  asks which side governs.

**Three further items were carried into a draft of this section from the session handoff and are
struck here, because re-measuring them at `41d1baa5e` is what the rule requires and two of the
three did not survive it.** They are recorded as struck rather than silently dropped, so the next
reader does not restore them:

- *"`design/04` §6 owes the `replaced_by_sku_id`-cleared-for-the-parent correction"* — **paid.**
  `inst-lc-undeprecate` now reads *"for the parent and for every child leg the reversal touches —
  the refusal names them"* under **P-D-49**.
- *"`design/05` §3.2 prose says eight grants, fourteen routes"* — **not present.** No
  `<n> grants, <n> routes` prose exists anywhere in that file; the **P-D-50** pass rewrote the
  paragraph, which now states *"A cell is per action"*. There is nothing to correct.
- *"`design/05` §6 bullet 23 is stale against its own C3"* — **unverifiable as addressed.** §6's
  bullet numbering has moved, and no bullet at that position names C3 in the way the claim
  describes. Restating it would assert a defect this document has not measured.
