<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md | Owners: BSS Product Catalog team -->

# Product & SKU Registry — Design Set


<!-- toc -->

- [Slice documents](#slice-documents)
- [Slice map (PRD §6 ↔ implementation phase)](#slice-map-prd-6--implementation-phase)

<!-- /toc -->

This folder holds the products-gear technical design as a **set of slice designs**: a shared
**Registry Foundation** ([`01-foundation.md`](./01-foundation.md)) plus per-capability handler
designs. Every slice **publishes through** the Foundation — the `Product`/`SKU` entity model,
identity and `skuCode` reservation, the revision-vs-published-version pair, the
draft→publish→(deprecated↔)→retired state machine, the fail-closed registered-validator
pipeline, append-only history, idempotency/ETag, the broker-native event fan-out (P-D-01), and
the audit trail. The Foundation owns no capability policy (it does not know what a `PlanTier`,
a metering unit, or a freeze participant is); each slice authors draft state, registers its
validation rules and read-model fields, and calls the Foundation publish API.

**The canonical index — architecture overview, phased slice map, dependency order,
cross-cutting normative statements, traceability — is [`../DESIGN.md`](../DESIGN.md).**
Requirements live in [`../PRD.md`](../PRD.md); decisions in [`../DECISIONS.md`](../DECISIONS.md)
(P-D-NN; joint contracts D-46/D-47 in the pricing register).

## Slice documents

- [`01-foundation.md`](./01-foundation.md) — **shared engine**: entity model + identity
  (`skuId`, atomic `skuCode` reservation), revision vs published version, lifecycle state
  machine core, fail-closed validation pipeline (registered validators), append-only history +
  diff, tenant isolation, idempotency/ETag, broker-native event fan-out + outbox, audit —
  complete and append-only, over a **reserved, unwritten** platform-sealing seam (P-D-08: no
  in-gear hash chain). (PRD §6.1 core, §6.5 core, §6.7 idempotency/eventing, §6.13 concurrency
  doors)
- [`02-taxonomy-attributes.md`](./02-taxonomy-attributes.md) — Category tree as governed live
  entities (cycle/depth/uniqueness-in-parent, per-tenant writer lock), the **governed-live-op**
  pattern (pinned operation envelope through the slice-05 gate), attribute definitions
  (deprecate-then-remove) + localized values + the `(locale, region, brand)` fallback chain,
  content-PII write-block hook, ungoverned metadata map (P-D-06 placement), well-known display
  seeds incl. `imageUri`/`unitDisplayLabel`/`marketingFeatures[]`. (§6.2, §6.4)
- [`03-sku-classification.md`](./03-sku-classification.md) — SKU typing with per-type
  `TypeProfile`s (`bundle` exempt from codes, override-gated per P-D-02), `sellable` (D-46),
  `PlanTier` taxonomy + stable tier codes, accounting codes, the generic `RecognizedSet`
  (units/codes/tiers — governed via `GovernedLiveOp`, removal operand = published heads +
  frozen content), metering-unit declaration as an atomic `(unit, usageTypeRef)` pair with
  publish-time resolvability (P-D-05, fail-closed on collector outage), unit de-listing. (§6.3)
- [`04-lifecycle.md`](./04-lifecycle.md) — deprecation/un-deprecation with `direct`/`cascaded`
  provenance (parent reversal revives only `cascaded`), scheduled transitions via the
  `ActivationRunner` (pinned approval, fail-closed re-validation, `deferred` retirement flips
  under the D-47 guard), `CascadePlan` + `DeferredRetireIntent` (cascades partial by design),
  `replacedBy` SoR, scope containment final rule + narrowing guard, v1 EOL lockout. (§6.5)
- [`05-governance.md`](./05-governance.md) — `MaterialityEvaluator` (bucket-registry-driven +
  enumerated ops + affected-entity trigger; the policy's own mutation is material), the
  approval workflow with a **stored** content snapshot (never re-derived — the pricing lesson),
  quorum by **principals** (the tenant's configured `N`, default 2 floor 0 — P-D-11; FinanceReviewer predicate binding at every `N >= 1`; the shorthand's reach enumerated by P-D-13). *This line read "author + ≥2 approvers" until 2026-08-26 — the fixed count P-D-11 retired, in the very phrasing that decision names as the origin of the floor being three without anyone deciding it.* The list continues: the P-D-02
  `OverrideCeremony`, one-shot approval consumption, the studio-inbox queue envelope
  (merge-compatible with pricing's), RBAC catalog, time-boxed read-only break-glass. (§6.7, §6.8)
- [`06-catalog-version.md`](./06-catalog-version.md) — the `CatalogVersion` machine: D-47 lanes
  with a per-tenant coalescer (interactive ≤ 5 s / bulk ≤ 5 min, gapless counter-row ids),
  `SnapshotBuilder` with stage-vs-commit re-validation (AC #40) and canonical checksum,
  `IntentfulResolver` (browse vs posted; a force-completed version stays **refused for posted
  use** until every forced participant freezes or releases, or the operator opts in — P-D-19),
  the `FreezeLedger` (acks, fail-closed timeout naming silent participants, force-completion
  pinning `not_frozen`, per-version participant-set snapshot), freeze-registration records as the
  AC #44 liveness source with liveness ending by an explicit `catalog_version × release`
  (P-D-18), grandfathering invariant, `compositionPending` clearing (a `system_signal` approval
  subject over a clean head — P-D-14, not an exemption), version-binding-at-freeze diff surface
  (AC #20a). (§6.6, §6.13)
- [`07-reference-signal.md`](./07-reference-signal.md) — the `WatermarkDoor` (S2S, monotonic,
  atomic full-set replacement), the 3-state `ReferencePredicate` with per-producer detail (+ the
  `no_producers` fail-safe), producer registration (P-D-03; symmetric capture-store ride), the
  bucket-ii `CorrectionDoor` (fresh-zero + the `N`-governed 05 quorum with `quorumReduced`
  recorded — P-D-13's fourth site, swept 2026-08-26 — + re-publish with `usageTypeRef`
  re-resolution), the **third admission arm** for an unresolvable meter target (P-D-16), the
  flag-gated break-glass correction (P-D-13's sixth site) + `TripwireCounter`. (§6.1, §6.13)
- [`08-read-models.md`](./08-read-models.md) — the event-driven `ReadProjector` over frozen
  versions + the 06 capture store (never head rows), per-state `VisibilityFilter` at query
  build, the `StalenessStamp` on every response (degraded included), per-tenant-partition
  shedding, bootstrap rebuild on checkpoint-behind-tail, re-parent subtree re-filing, the
  deferred-intent/freeze dashboards; rebuildable-not-records storage exemption. (§6.8, §7)
- [`09-bulk-promotion.md`](./09-bulk-promotion.md) — the `BulkBatch`/`RowLedger` pipeline
  (stage-as-drafts through the ordinary doors, aggregated `ChangeReport` as the stored approval
  snapshot, per-row commit pinned to ledger revisions under the batch composite act,
  ONE CatalogVersion via 06's `operation_key` with an explicit close marker, row-level domain
  events emitted + ONE coalesced summary), deterministic export from the 06 manifest, `PromotionResolver` (create / no-op / update-as-draft / conflict over the C5
  identities), p2 bulk lifecycle with per-SKU flip guards intact. (§6.9)
- [`10-retention-erasure.md`](./10-retention-erasure.md) — the `IdentityRefMap` (the gear's one
  PII table; erasure = tombstone the map, immutable records untouched — one mechanism, two
  triggers), the `PiiDetector` policy + the curated allow-list behind 02's hook (base quorum
  plus a mandatory recorded Legal sign-off reference — no gear-side Legal role, P-D-10), retention
  clocks to statutory max with the fail-closed AC #44 `RetentionGate` over 06's liveness
  records (+ derived entity-version retention), the NFR #5 restore drill re-verifying 06
  checksums byte-for-byte. (§6.11)
- [`11-clone.md`](./11-clone.md) — the `CloneDoor` through the ordinary 01 create door, the
  normative `DispositionTable` (copy / reset / copy-and-re-validate per field class; pricing
  never), the revival-rename rule resolving the P-D-04 interaction (quasi-code renames, the
  storefront doesn't), PII re-screen on copy, `clonedFrom` lineage, per-child ledger. (§6.10, p3)
- [`12-consumer-contracts.md`](./12-consumer-contracts.md) — the `SeamSuite` (SchemaPin over the **eight** obligation operands
  plus `CatalogVersion` as a **surface** — membership is P-D-12's rule, not a list; joint
  fixtures grown per the authorable-once-counterpart-AC-exists rule), the `ObligationRegister` (every consumer duty: asserted or explicitly OWED — the
  watermark fixture first), event versioning vN→vN+1 + the replay/bootstrap contract (08's
  projector as its first consumer), the SDK/§9 surface incl. the `CatalogSku`-superset shape
  and the studio-inbox envelope assertion, and the **nine** `CoverageChecks` doc-lints (requirement
  coverage, AC #38 map, door×grant pairing, event bookkeeping, register hygiene, id uniqueness,
  identity materialization, no-monetization-marker, obligation×pin coupling). (§6.12, §9)

## Slice map (PRD §6 ↔ implementation phase)

See [`../DESIGN.md`'s design-set table](../DESIGN.md#design-set-ordered-by-implementation-phase)
— the canonical table with phases and dependencies. (The old target, `#13-slice-map-phases-dependency-order`,
never existed: DESIGN.md §1.3 is "Architecture Layers" and the slice map is a `####` block —
item 27 of the 2026-08-26 review, the only broken anchor in the products doc tree.) The numeric prefix is implementation order, not the PRD
subsection number.
