<!-- CONFLUENCE_TITLE: [BSS]: Rating — Decision Register -->
<!-- Related: ./DESIGN.md, ./PRD.md, ./SEAMS.md | Owners: BSS Rating team -->

# Rating — Decision Register

<!-- toc -->

- [Decision register](#decision-register)
- [Open items](#open-items)
- [Source](#source)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-rating-decisions`

> Rating design decisions (rows T-D-01…T-D-15 were authored under the gear's historical name
> "Tariffs" — see T-D-16 / ADR-0002). The load-bearing early decisions are **inherited from the
> cross-gear seam analysis** with the pricing gear (2026-07-10) and recorded verbatim in
> [`SEAMS.md`](./SEAMS.md); this register is the gear-local index over them plus decisions taken
> during Design. Seam ids (K/O/S/W/M/P/F/B/C/G/N) resolve into [`SEAMS.md`](./SEAMS.md).

## Decision register

| # | Decision | Source seam | Status |
|---|----------|-------------|--------|
| T-D-01 | Adopt the pricing 8-axis canonical scope key for selection + non-overlap; `phase` is `phase_id`; multi-generation grandfathering selects by the pinned price id's `cohort`. | K1-K5 | **adopted 2026-07-10** |
| T-D-02 | Step-4 overlays **stack** all survivors; class-specificity order breaks ties, not exclusivity. | O1-O3 | **adopted 2026-07-10** (pricing to confirm tie-break-not-exclusivity) |
| T-D-03 | One `pricingSnapshotRef`, three writers; Tariffs is the composition SoR. | S1 | **adopted 2026-07-10** |
| T-D-04 | Open-period re-resolution replays strictly from the pinned snapshot (no live catalog read); tier-counter key `(subscription, meter, dimensionKey, window)`. | W2, M7 | **adopted 2026-07-10** |
| T-D-05 | `modelKind = {flat, per_unit, graduated, volume, package}`; per_unit, package, and composite/derived meter are in launch; Volume Variant B not authorable; hybrid/committed are compositions. | M1-M5 | **adopted 2026-07-10** |
| T-D-06 | Single pricing Slice 5 approval engine; Tariffs registers fail-closed validators, not a second workflow; ledger `dual_control` stays separate (different bounded context). | G1 | **adopted 2026-07-10** |
| T-D-07 | `prorationBasis` (incl. `none`) and `billingAnchorPolicy` + D-20 clamp adopted verbatim; enum-drift CI gate. | P1, P2 | **adopted 2026-07-10** |
| T-D-08 | Usage rows phase-invariant by default (phase-specific wins); reserved-rate two-source split; effective rev-share pass-through for bundle summing. | F1, M9, B1 | **adopted 2026-07-10** |
| T-D-09 | `pricingSnapshotRef` gains the **eighth named segment `commitmentReservation`** (reservation match: id, flavor, `reservedQuantity`, rate source; pool set: per-pool id, unit, `poolType`, balance @ `balanceVersion`, draw order, rollover; reserved-vs-pool split); writer Tariffs @ eval — closes the S1 segment-naming residue. | S1 | **adopted 2026-07-11** |
| T-D-10 | Commitment balance lifecycle: Rating publishes per-outcome `CommitmentBalanceEffect`s (idempotent on the outcome's evaluation/correction key); Contracts serializes per-pool `balanceVersion`; a balance-affecting correction cascades delta-only re-resolution of later-`balanceVersion` units that drew (or were rated overage against) the pool. | M8 → design/05 §4.1 | **adopted 2026-07-11** (cross-PRD: mirror in Contracts; the Rating side is intra-gear since T-D-16 — pipeline slice 15) |
| T-D-11 | Delta-dedup owner = **Rating** — persists delta outcomes by correction key; retried keys return the recorded outcome; Billing consumes Adjustments idempotently (defense in depth). | design/01 §2.2, design/08 §2.2 | **adopted 2026-07-11** |
| T-D-12 | Windowed models across intra-window boundaries: per-slice sub-window evaluation units with the frozen **`bandOffsetQ`**; window-activation/phase-conversion boundaries always carry (pricing `inst-tb-window-continuity`), plan-change per carry-vs-reset flag; volume selects the band by window-cumulative `Q`; package counts blocks by cumulative ceil-diff; the correction key gains the slice coordinate `(window[, slice], prior-rated-version, snapshot)`. | M7, P2 → design/03 §4.3 | **adopted 2026-07-11** |
| T-D-13 | Step-6 recompute contract: a consumption-reservation split re-runs **steps 3–5 as a unit** over the on-demand remainder (re-band from zero; the frozen overlay stack + contract overlay re-apply); the reserved portion is not re-overlaid. Tier-counter exclusion + pool asymmetry (pricing `inst-rv-tier-q`) registered in PRD §6.6 / AC 19. | M9 → design/05 §4.2, design/04 §4.2 | **adopted 2026-07-11** |
| T-D-14 | Commitment pool flavors: frozen `poolType ∈ {prepaid_drawdown, committed_rate}` — billability per flavor (due-zero + notional lineage vs in-arrears at the committed rate) and normative shortfall true-up formulas (quantity/spend basis); the commitment sale itself bills outside PLAL (Contracts/Billing at sale). | M8 → design/05 §4.1/§4.5 | **adopted 2026-07-11** |
| T-D-15 | Period-driven evaluation units (recurring lines, capacity-flavor charges, true-up surfacing) keyed `(subscription, priceId, chargeKind, lineKey, AnchorPeriod)`, synthesized by **Rating's period tick** at anchor boundaries — no Tariffs scheduler; a zero-usage period still emits its period-driven units. | design/01 §4.2, design/11 §4.1 | **adopted 2026-07-11** |
| T-D-16 | **Gear consolidation**: Tariffs (evaluation core) and the Rating pipeline become **one `rating` gear** — the core as a no-I/O `rating-core` crate (evaluation slices 01–11 survive as-is), pipeline slices 12–16 absorb the empty rating-engine scope (VHP-810); "tariff" returns to the Pricing vocabulary (rate definitions); T-D-10/11/15 become intra-gear. Full naming + migration map: [`ADR/0002`](./ADR/0002-cpt-cf-bss-rating-adr-rating-gear-consolidation.md). | ADR-0002 | **accepted 2026-07-11** (single owner); commits A–B landed, residues tracked below |
| T-D-17 | **Level-based (gauge) aggregation via granule fold** (joint with pricing D-44, supersedes the F-40 "sum-only launch"): for `aggregationFunction ∈ {peak, time_weighted}` the meter consumes point-stamped gauge samples; the window cuts into frozen `aggregationGranularity ∈ {hour, day}` granules, each folds deterministically (max / step-integral with bounded `hold_last`, gap → 0 + operator signal, never a guess), and window `Q` = **Σ granule folds — additive by construction**, so M7 single-writer key, supersession continuity, T-D-12 `bandOffsetQ`, band/package math, and delta-only corrections are untouched; a late sample re-folds only its granule (qVersion++). Billable unit = level·granule-hours (SKU-declared), distinct from the sample's level unit. No composite co-occurrence at launch. Product drivers: cloudlet peak-per-hour, storage GB-month. | pricing D-44; PRD `fr-level-aggregation` §6.5/§17.1; design/03 §4.3, design/12 §2.2, design/13 §4.1; SEAMS M10/UC6 | **adopted 2026-07-16 (product call, confirmed)** |

## Open items

- **O3** — pricing-side confirmation that "most-specific-wins" is a tie-break, not overlay exclusivity.
- **M6** — dimensional launch posture wording (declaration in-scope now; value-pricing OSS-gated).
- **Pre-existing (vendored PRD):** §5.1 acceptance-criteria cross-references drifted from §12 numbering (max AC 20) — reconcile during Design.
- **From the 2026-07-11 slice review** (details: [`SEAMS.md`](./SEAMS.md) technical residues): composite × dimensions input-join rule (design/03 §3.6); mixed-settlement coupon rules — fail-closed meanwhile (design/06 §4.2); seat-change boundary transport with Subscriptions (design/09 §4.3); bundle-level coupon attachment (design/10 §4.5). Foundation opens: DECIMAL precision (with Billing), clamp-vs-credit (PRD §15). *Resolved 2026-07-11:* S1 segment naming → T-D-09; delta-dedup owner → T-D-11; balance write-back → T-D-10.
- **From the 2026-07-11 tri-review** (to close before Design lock): coupon equal-benefit tie-break (design/06 §4.2 — default proposal: ascending `couponId`); `whole_unit` attribution on a split (design/09 §4.1); validator-4 enforcement point — Contracts-side publish gate (design/10 §4.4); validator packaging — shared crate vs pricing-side impl (design/10); RatedCharge/BillableItem mapping ratification by the pipeline design (design/11 §4.1 → pipeline slice 15 — intra-gear since T-D-16).
- **Registry seams (`products` gear — PRD vendored 2026-07-16 to `gears/bss/products/docs/PRD.md`, from PR #4177):** RG1 freeze-protocol composition (registry `freezeComplete` × pricing pin/warm-completion × rating W2 replay — rating is not in the freeze-participant set) — **still open**; RG2 single-unit-per-SKU vs dimensional lines — **closed** (fix applied in the vendored copy); RG3 pre-consolidation naming — localization debt recorded in the vendored PRD's provenance note. Tracked in [`SEAMS.md`](./SEAMS.md) §I.
- **Migration residues (ADR-0002 / T-D-16):** prose re-voicing of the evaluation-core slices (historical "Tariffs" → the core, neighbour-"Rating" → the pipeline) — bridge notes in [`DESIGN.md`](./DESIGN.md) / [`SEAMS.md`](./SEAMS.md) apply meanwhile; the `Tariffs`/`PLAL` → `Rating`/`rating-core` prose rename is **done in the living docs** (design slices 01–16, DESIGN.md, README, PRD.md — *Done 2026-07-16*); this register, [`SEAMS.md`](./SEAMS.md), and the ADRs deliberately retain the historical "Tariffs"/"PLAL" naming as dated decision/analysis records (each carries a bridge note). The **pricing-side Commit C** ("Tariffs" prose in `gears/bss/pricing/docs`) remains the one open cross-gear rename. (Upstream vhp-architecture is legacy provenance, not maintained — no propagation obligation for the manifest §4.2 note or the `PRD-rating-engine-202604031200` marker.) *Done 2026-07-11:* artifacts-board `/rating` alias (page retitled; `/tariffs` kept as an alias). *Done 2026-07-15:* pipeline slices 12–16 authored beyond skeleton (full §1–§5); the slice-11 §4.1 handoff mapping ratified intra-gear by slice 15 (finding C6 closed).

## Source

Full seam analysis, rationale, ownership matrix, and rejected alternatives are in
[`SEAMS.md`](./SEAMS.md) (4-agent cross-gear pass, 2026-07-10). Requirements in
[`PRD.md`](./PRD.md).
