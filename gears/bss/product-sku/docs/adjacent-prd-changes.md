# Adjacent-PRD changes — counterpart requirements for the product-sku registry

<!-- Purpose: the registry PRD (./PRD.md) declares inbound contracts that other modules
     must produce, and decisions recorded here (§15) that need mirror lines in the
     producing PRDs. Those PRDs currently live on open PRs — this file holds the
     ready-to-transfer text so the changes can be applied once they merge.
     Each section: target document → insertion point → paste-ready text → source
     (registry PRD anchor). Placeholders that await ratification are marked ⟨TBD⟩. -->

Status: pending transfer. Source of truth for the registry side: [`./PRD.md`](./PRD.md)
(§9.2 inbound contracts, §15 gates) and [`./DESIGN.md`](./DESIGN.md) (§5.10 signal
endpoints). Registry-side decisions referenced below were recorded 2026-07-17.

---

## 1. `PRD-plan-price-modeling-202605281200`

**Insertion point:** next to the "CatalogVersion increment contract (with registry)"
section (~line 363), or as a new subsection in the integration/System-Boundaries area.

### 1.1 New subsection — outbound obligations to the registry

```markdown
#### Registry integration obligations (outbound)

- On every `PlanPublished`, this module MUST submit an idempotent, tenant-scoped
  **CatalogVersion publish-request** to the registry
  (`POST /v1/catalog-version-publish-requests`; registry AC #46 /
  `cpt-cf-bss-product-sku-contract-catalog-publish-request`). The registry is the
  sole incrementer and MAY batch; the delay from the oldest pending request to
  `CatalogVersionPublished` is bounded by the ratified max batching-delay SLO.
- This module MUST acknowledge every `CatalogVersionPublished` it participates in
  (**freeze-ack**, registry AC #21–#23) after freezing its plan/price/descriptor
  content for that `catalogVersionId`; the acknowledgment names this module and MUST
  be sent under this module's own service identity (the registry rejects mismatches
  with `SIGNAL_IDENTITY_MISMATCH`).
- This module MUST publish the **`SkuReferenceCount` per-producer watermark**
  (registry AC #3): "as of `T`, my complete live-reference set is {…}" — monotonic
  `as_of`, explicit tenant scope, refresh interval within the registry's freshness
  threshold (interim 15 min).
- On completing a bundle's composition, this module MUST emit the
  **composition-completed signal** (registry AC #25) so the registry clears
  `compositionPending` and re-opens adoption.
```

### 1.2 Close the open-question row (~line 1305)

Replace "Still open with Registry: exact trigger taxonomy + the max batching-delay
SLO value" with the ratified values:

```markdown
Answered 2026-07-17 — max batching-delay SLO = ≤ 15 min (PRD-product-sku §17.1,
ratified); trigger taxonomy = the base rule only: pending publish-requests older than
the policy age trigger one discretionary catalog publish (registry
`PublishRequestBatcher`); mass repricing coalesces into the same batch.
Owner: Registry + plan-price.
```

### 1.3 System-Boundaries row (~line 477)

Add to the "produces →" side of the Catalog-registry row: `CatalogVersion`
publish-requests, freeze-acks, `SkuReferenceCount` watermarks, composition-completed
signals.

---

## 2. `PRD-contracts-agreements-202601120119`

**Insertion point:** integration/System-Boundaries section (wherever the catalog
registry dependency is described).

### 2.1 Outbound obligations

```markdown
#### Registry integration obligations (outbound)

- This module MUST acknowledge every `CatalogVersionPublished` it participates in
  (**freeze-ack**, registry AC #21–#23) after freezing the catalog-derived content of
  its quotes/contracts for that `catalogVersionId`; sent under this module's own
  service identity.
- This module MUST publish the **`SkuReferenceCount` per-producer watermark**
  (registry AC #3): complete live-reference set, monotonic `as_of`, explicit tenant
  scope, refreshed within the registry freshness threshold (interim 15 min).
```

### 2.2 Two decisions Contracts must record (registry pre-approval inputs)

```markdown
- **Draft/quote references**: draft and quote references **DO count** toward
  `referenced` in the watermark; the semantics apply identically to the registry's
  mutability, correction, and retirement decisions (registry AC #3).
  Decision recorded 2026-07-17.
- **Re-present-vs-accept at freeze**: **re-present** — a bound-but-not-yet-frozen
  quote that re-resolves to a newer published version at freeze is re-presented to
  the customer, never silently accepted (registry AC #13; the comparison surface is
  `GET /v1/catalog-versions/{id}/entries` + the version-diff API).
  Decision recorded 2026-07-17.
```

---

## 3. Billing PRDs (`PRD-billing-module-202601120119`, `PRD-billing-ledger-balances-202604041200`)

**Insertion point:** the section describing consumption of `CatalogVersion` /
descriptor snapshots.

```markdown
#### Registry integration obligation (outbound)

- This module MUST acknowledge every `CatalogVersionPublished` it participates in
  (**freeze-ack**, registry AC #21–#23) after freezing the descriptor content it
  consumes for that `catalogVersionId`; sent under this module's own service
  identity. Posting against a version is allowed only once the version is
  posting-safe (`freezeComplete`, or force-completed with this module's content
  explicitly pinned fail-closed).
```

---

## 4. Marketplace PRD (`PRD-product-catalog-marketplace-202601120119`, or its Marketplace-only successor)

**Insertion point:** the listings/vendor-portal section that references published SKUs.

```markdown
#### Registry integration obligation (consumer-side)

- Marketplace listings do **not** count toward the registry's `referenced` predicate
  (registry decision 2026-07-17, PRD-product-sku §15). Instead, this module MUST
  consume `SkuImmutableFieldCorrected` and re-validate/re-synchronize every listing
  that references the corrected SKU (asserted by the registry↔plan-price seam suite,
  registry AC #36). Registering as a full `SkuReferenceCount` producer remains
  possible later without registry schema change.
```

---

## 5. Canonical mirror `PRD-product-sku-management-202606101924` (vhp-architecture)

Not an adjacent module — the same requirements in the architecture-repo format.
Sync all registry-side changes of 2026-07-17 so cross-PRD citations stay valid
(the mirror numbers ACs continuously 1–56; our new AC #46 becomes its **AC #57**):

- publish-request: FR + §9.2 contract + AC #46/#57; §5.2 re-scope; §17.1 batching-delay row;
- identity binding + explicit tenant on S2S signals (AC #3/#21 clauses);
- fix-pass changes: correction race-window text + re-check, un-deprecate cancels
  scheduled retirement, composition-signal lifecycle semantics, deprecated
  accounting-code handling, deprecated-entity mutability, `replacedBy` target
  tracking, materiality alignment to §17.1, per-brand default-locale, snake_case
  retirement payload, Risks-table citation retargeting;
- recorded decisions: person-name allow-list stays (option A), Marketplace listings
  excluded from `referenced` (option B), identifier-scheme deviation accepted;
- §15: four pre-approval gates (SkuReferenceCount; region algebra; freeze-ack +
  composition owners; publish-request SLO) + answered rows; §17.1 freeze-timeout row.

---

## Transfer checklist

| # | Target | Owner | Blocked by | Done |
|---|--------|-------|-----------|------|
| 1 | plan-price PRD (obligations + SLO answer) | plan-price + Registry | target PRD merge (decisions ratified 2026-07-17) | ☐ |
| 2 | Contracts PRD (obligations + 2 decisions) | Contracts | target PRD merge (decisions ratified 2026-07-17) | ☐ |
| 3 | Billing PRD(s) (freeze-ack) | Billing | target PRD merge (commitment + delivery milestone recorded 2026-07-17) | ☐ |
| 4 | Marketplace PRD (re-sync obligation) | Marketplace | target PRD merge | ☐ |
| 5 | Canonical mirror sync | Registry/Architecture | — | ☑ 2026-07-17 |
| 6 | Registry PRD §15 gates → owner + date; §17.1 final SLO | Registry | — (all commitments + common delivery milestone "registry v1 GA" recorded 2026-07-17) | ☑ 2026-07-17 |
