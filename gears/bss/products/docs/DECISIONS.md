# DECISIONS — Product & SKU Registry (`products` gear)

Decision register for the products gear. Prefix **P-D-NN**. A decision lands here with its
date, rationale, and the propagation list of every document that restates it; the §15 rows of
[`PRD.md`](./PRD.md) that answered a gate cite these numbers rather than carrying the only copy.
Historical context: D-46 (`sellable`) and D-47 (`CatalogVersion` increment taxonomy) predate this
register and live in the **pricing** register (`gears/bss/pricing/docs/DECISIONS.md`) — they are
joint contracts, cited from here by their pricing numbers, never duplicated.

<!-- toc -->

- [P-D-01 — Broker-native event envelope (not CloudEvents 1.0)](#p-d-01--broker-native-event-envelope-not-cloudevents-10)
- [P-D-02 — CatalogVersion increments are mechanical; governance at entity publish](#p-d-02--catalogversion-increments-are-mechanical-governance-at-entity-publish)
- [P-D-03 — SkuReferenceCount v1 producer set = {pricing}](#p-d-03--skureferencecount-v1-producer-set--pricing)
- [P-D-04 — Absolute product-name uniqueness (region-independent)](#p-d-04--absolute-product-name-uniqueness-region-independent)
- [P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding](#p-d-05--usagetyperef-validates-resolvability-only-uc3c-lives-at-the-pricing-meter-binding)
- [P-D-07 — The staleness stamp is a floor, advanced only when its content is present](#p-d-07--the-staleness-stamp-is-a-floor-advanced-only-when-its-content-is-present)
- [P-D-06 — Metadata map lives outside frozen version content](#p-d-06--metadata-map-lives-outside-frozen-version-content)

<!-- /toc -->

## P-D-01 — Broker-native event envelope (not CloudEvents 1.0)

- **Date**: 2026-08-25 (product call; PRD §15 gate "Event-envelope conformance")
- **Decision**: the registry publishes its events in the platform event-broker's **broker-native
  schema** (`gears/system/event-broker`, its ADR-0003) — **not** CloudEvents 1.0. The semantic
  obligations are envelope-agnostic and bind unchanged: versioned (semver) schema references
  (the broker-native equivalent of `dataschema`), `vN`→`vN+1` consumer compatibility,
  correlation/causation, per-aggregate ordering keys `(tenant, aggregate)`, pseudonymous actors.
- **Why**: the manifest §7.2 CloudEvents mandate predates the built broker, whose ADR-0003
  explicitly rejects CloudEvents wire conformance (no `dataschema` field). A mapping layer would
  be a second envelope to keep honest with zero consumers asking for it.
- **Residue owed**: manifest §7.2 amendment re-scoping the CloudEvents mandate (owner:
  Architecture / Common Core).
- **Propagated**: PRD §2, §4.1, §5.1, `fr-registry-eventing-audit`,
  `fr-event-versioning-replay`, §9.2, AC #28/#29; design slice 01 §4 (event fan-out).

## P-D-02 — CatalogVersion increments are mechanical; governance at entity publish

- **Date**: 2026-08-25 (product call; PRD §15 gate "D-47 demand-driven increments vs publish governance")
- **Decision**: a `CatalogVersion` increment — operator- or system-initiated (pricing D-47
  lanes: interactive ≤ 5s coalescing, bulk ≤ 5 min) — is **mechanical**: it snapshots only
  content whose governance already happened, and is never itself an approval gate. Every
  governance gate attaches to the **entity publish** that introduces the exception; specifically
  the uncomposed-`bundle` two-person override moves from `CatalogVersion` publish to the
  bundle's entity publish (the lint findings presented to its approvers). The
  `CatalogVersion`-publish lint is an informational report for operator publishes.
- **Why**: the same FR carried both "manual two-person publish with blocking-with-override" and
  D-47's "increment within 5 seconds of a downstream publish request" — a machine cannot wait
  for two humans, skipping them breaks governance, and refusing breaks the ratified SLO. Moving
  the override to the human act dissolves the contradiction without weakening either control.
- **Propagated**: PRD §4.1/§5.1/§5.2, `fr-define-sku`, `fr-catalog-version-publish`,
  `fr-bundle-adoption-guard`, `fr-prepublish-lint`, AC #7/#19/#25/#45; design slices 03 (`inst-cl-bundle-override`), 05, 06.

## P-D-03 — SkuReferenceCount v1 producer set = {pricing}

- **Date**: 2026-08-25 (product call; PRD §15 gate "SkuReferenceCount owner + delivery date")
- **Decision**: the v1 **registered producer set** of the `SkuReferenceCount` per-producer
  watermark is **{plan-price (pricing gear)}**, built jointly with this gear's development and
  delivered before products v1 GA. Subscriptions and Contracts register as producers at their
  own build time; their GA is gated on producing the signal. Until the pricing watermark ships,
  AC #2/#4/#18 run fail-safe (break-glass + tripwire).
- **Why**: pricing is the only coded counterpart and already holds the data (live plan→SKU
  references); `fr-reference-producer-registration` makes late onboarding safe by construction
  (unregistered silence pins nothing, registration never re-flips history). One-party
  commitment instead of a three-party negotiation with two docs-only gears.
- **Propagated**: PRD §9.2, §14, `fr-reference-producer-registration`, AC #43; pricing PRD §15
  (mirrored answered row); rating SEAMS ownership matrix; design slice 07.

## P-D-04 — Absolute product-name uniqueness (region-independent)

- **Date**: 2026-08-25 (product call; PRD §15 gate "Region-set algebra")
- **Decision**: product-name uniqueness on `(tenantId, brandId, normalized(name))` is
  **absolute** — two same-named Products under one tenant+brand are forbidden regardless of
  region scope. The canonical internal name is a quasi-code; localized display names are
  attributes and repeat freely (regional variants: distinct internal names, identical display
  names). Region-set semantics survive only for **parent-child scope containment**
  (`fr-parent-child-integrity`) — pinned in slice 01/04 Design, interim conservative
  subset-check fail-closed.
- **Why**: the overlap/disjointness algebra was a pre-approval gate with real false-reject and
  false-allow risk; absolute uniqueness deletes the whole question from the create door.
  Strict→loose later is a compatible widening; loose→strict would be a breaking migration.
- **Propagated**: PRD glossary (Region), `fr-create-product`, `fr-expected-failure-behavior`,
  AC #5, AC #33a (the promotion fallback identity), AC #38, §16; design slices 01 (uniqueness index + `normalized(name)` pin), 02 (display-name coexistence), 04 (containment), 09 (C5 promotion identity).

## P-D-05 — `usageTypeRef` validates resolvability only; UC3(c) lives at the pricing meter binding

- **Date**: 2026-08-25 (veto round over the UC3 adoption block of 2026-07-28)
- **Decision**: registry publish validates that a declared `usageTypeRef` **resolves** in the
  usage-collector's platform-global UsageType catalog — nothing more. "And is active" is dropped
  (a UsageType carries no lifecycle state: register/get/list/delete only, deletion FK-guarded
  against usage records — not against catalog meters, which rating's quarantine rule
  fail-safes). The UC3(c) dimension-set cross-validation is **not** performed here (the registry
  assigns dimension sets to plan-price and holds no operand): its home is pricing
  `inst-cmp-usagetype` (confirmed 2026-07-31, built) — priced `dimensionKey` **⊆** the
  UsageType's `metadata_fields` at plan publish.
- **Why**: both original clauses named operands that do not exist registry-side; subset (not
  equality) is the load-bearing invariant — pricing fewer dimensions than the source emits is
  harmless, pricing one it never emits is the hazard.
- **Propagated**: PRD `fr-metering-unit-declaration`, AC #8, §15 (answered row); rating SEAMS
  UC3 row + ownership matrix; pricing design/02 (stale "registry holds equality" premise
  retired); design slice 03.
- **Residue (2026-08-25, PR #14 review)**: quarantine-on-deleted-UsageType is a fail-safe, not
  an operating mode — the deletion-guard/deletion-signal negotiation with usage-collector is a
  PRD §15 open ("UsageType deletion vs published declarations").

## P-D-07 — The staleness stamp is a floor, advanced only when its content is present

- **Date**: 2026-08-26 (design slice 08; premise corrected by its own review, H1/H2)
- **Decision**: `asOfCatalogVersion` on every read response is a **floor**: every catalog
  version ≤ the stamp is fully reflected in the projection, and later entity events may add,
  change, or **remove** content relative to the stamped version (`projectedAt` is the
  fine-grained coordinate; `null` + `projectedAt` before a tenant's first version). The stamp
  advances only after the `CatalogVersionPublished` changed-entity list is projected from
  frozen rows in the same step — it never claims a version whose content the projection lacks.
- **Why**: the PRD's one-signal rule binds staleness to resolvable catalog versions, but entity
  events between versions are not additive (a retirement flip removes content without an
  increment) — an "as of CV N" claim over mutated content would lie in the dangerous direction.
- **Propagated**: design slice 08 (`inst-rp-stamp`, §6); compatible with PRD
  `fr-cache-first-browse`/NFR #7 as written (no PRD edit).

## P-D-06 — Metadata map lives outside frozen version content

- **Date**: 2026-08-25 (introduced by design slice 02; **flagged for review** — first slice-born
  decision, not a §15 gate answer)
- **Decision**: the per-entity metadata map is stored **beside** the entity, outside the frozen
  `products_entity_version` content: mutable in place on any non-terminal entity **without a
  published-version bump** (audited, `MetadataUpdated`-evented per write); a `CatalogVersion`
  captures the map **as of its own snapshot instant**, and that copy freezes with the snapshot.
- **Why**: the PRD demands both "ungoverned free-form channel" and "captured in CatalogVersion
  snapshots" — putting the map inside version content would force a governed version bump per
  sync-marker write (contradiction one) or mutate frozen versions (contradiction two). Beside
  the entity, old snapshots stay byte-identical while the map moves freely.
- **Propagated**: design slice 02 §2 (`inst-md-placement`) + §4.1/§5; slice 06 owes the
  snapshot-capture step; PRD glossary "Metadata map" is compatible as written (no PRD edit).
