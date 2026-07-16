<!-- CONFLUENCE_TITLE: [BSS]: Plan & Price Modeling — Open Design Decisions (review wave 2026-07-09) -->
<!-- Related: ./DESIGN.md, ./PRD.md, ./design/ | Owners: BSS Product Catalog team -->

# Open Design Decisions — Plan & Price Modeling

<!-- toc -->

- [How to use this document](#how-to-use-this-document)
- [Status board](#status-board)
- [A. Design-lock blockers](#a-design-lock-blockers)
- [B. Governance](#b-governance)
- [C. Plan & price shape (S2/S3)](#c-plan--price-shape-s2s3)
- [D. Consumer contracts & windows (S6/S7)](#d-consumer-contracts--windows-s6s7)
- [E. Bundles, price overlays, primitives (S8/S9/S10)](#e-bundles-price-overlays-primitives-s8s9s10)
- [F. Lifecycle & operator efficiency (S11/S12)](#f-lifecycle--operator-efficiency-s11s12)
- [G. Ratifications — decisions already applied by the review fix wave](#g-ratifications--decisions-already-applied-by-the-review-fix-wave)

<!-- /toc -->

## How to use this document

Source: the 2026-07-09 design-review wave (7 parallel review passes over all 15 docs; the
~50 mechanical fixes were applied directly — this file holds only what needed an actual
decision). **Status: all 39 decisions and all 14 ratifications are closed as of 2026-07-10**
and propagated into the named docs; each entry records the decision, rationale, and the
propagation surface. Items marked *(autonomous)* were decided by the reviewing agent under a
standing mandate ("decide where the call is technical"); three carry product flavor and are
explicitly **flagged for veto**: D-23 (rule-based change targets cut from launch), D-34
(in-flight migration cancel added), D-39 (migration never grants a new trial). Reopening any
item = flip its status and record why.

Severity: **[H]** breaks money/correctness or is unimplementable as written · **[M]** teams
can build incompatible behavior · **[L]** contained.

## Status board

| # | Sev | Title | Status |
|---|-----|-------|--------|
| D-01 | H | S4 tax-completeness check has no data source | **DECIDED 2026-07-10** |
| D-02 | H | Second grandfathering cutover on the same scope key is impossible | **DECIDED 2026-07-10** |
| D-03 | H | Cutover atomicity across the external window SoR | **DECIDED 2026-07-10** |
| D-04 | H | Coverage hole between `grandfatherUntil` expiry and renewal re-bind | **DECIDED 2026-07-10** |
| D-05 | H | Retirement cancels the scheduled halves of an approved cutover | **DECIDED 2026-07-10** |
| D-06 | H | Overlay/membership read-model propagation has no publish trigger | **DECIDED 2026-07-10** |
| D-07 | H | Rev-share: exact shares vs tolerant shares (PRD AC #23 conflict) | **DECIDED 2026-07-10** |
| D-08 | H | `fixed`/amount PriceOverlay adjustments have no currency axis | **DECIDED 2026-07-10** |
| D-09 | H | Payer in multiple customer groups: resolution undefined | **DECIDED 2026-07-10** |
| D-10 | M | Threshold-policy mutation is itself ungoverned | **DECIDED 2026-07-10** |
| D-11 | M | Bundle publish authz: no role can execute the stated conjunction | **DECIDED 2026-07-10** |
| D-12 | M | Finance access to history/audit reads vs the role matrix | **DECIDED 2026-07-10** |
| D-13 | M | Historical-import governance (pipeline + second person) | **DECIDED 2026-07-10** |
| D-14 | L | G4 WORM vs hash chain: transactional-audit claim | **DECIDED 2026-07-10** |
| D-15 | H | Phase→price coverage rule missing (empty phase publishes) | **DECIDED 2026-07-10** |
| D-16 | M | Add-on dependency/conflict edges have no data model | **DECIDED 2026-07-10** |
| D-17 | M | Who sets `fail_closed_top` | **DECIDED 2026-07-10** |
| D-18 | M | Tiered non-usage rows: legal or forbidden | **DECIDED 2026-07-10** |
| D-19 | M | `phase` scope-key axis typing (literal vs `phase_id`) | **DECIDED 2026-07-10** |
| D-20 | M | `customEveryN Months(n)` anchor semantics | **DECIDED 2026-07-10** |
| D-21 | L | Save-time vs publish-time validation split (AC #12) | **DECIDED 2026-07-10** |
| D-22 | L | What the supersession-continuity fixture gates | **DECIDED 2026-07-10** |
| D-23 | M | Rule-based `allowedChangeTargets` defeat publish-time guarantees | **DECIDED 2026-07-10** |
| D-24 | M | Retired target left in the change graph | **DECIDED 2026-07-10** |
| D-25 | M | Cross-boundary change edges neither rejected nor marked | **DECIDED 2026-07-10** |
| D-26 | M | Must UC-side window mutations route through Slice 7 | **RESOLVED by D-03** |
| D-27 | M | Resolved grant-set drift after a registry tier-policy change | **DECIDED 2026-07-10** |
| D-28 | L | Multi-key (batch) cutover shape | **DECIDED 2026-07-10** |
| D-29 | M | Prepaid GA-gate mechanics (derive / scope / clear) | **DECIDED 2026-07-10** |
| D-30 | M | `ResolvedGroupFreezer` ownership (catalog vs Tariffs) | **DECIDED 2026-07-10** |
| D-31 | M | Retiring a plan targeted by a `PriceOverlay` | **DECIDED 2026-07-10** |
| D-32 | L | Composite-meter output-unit ownership | **DECIDED 2026-07-10** |
| D-33 | L | F-34 member-scoped preview as a tracked GA gate | **DECIDED 2026-07-10** |
| D-34 | H | Cancelling an `in_progress` migration | **DECIDED 2026-07-10** |
| D-35 | H | Bulk/repricing rows vs pending approval units on one key | **DECIDED 2026-07-10** |
| D-36 | M | Execution-time re-validation of locks + boundary deltas | **DECIDED 2026-07-10** |
| D-37 | M | Bulk-lock crash/timeout release path | **DECIDED 2026-07-10** |
| D-38 | L | Migration-cancellation propagation to Subscriptions | **DECIDED 2026-07-10** |
| D-39 | M | Migration entry phase on a phased target (trial re-entry) | **DECIDED 2026-07-10** |
| D-40 | M | `tierQualificationWindow` — trailing-tier qualification (third window) | **DECIDED (autonomous) 2026-07-12 · flagged for veto** |
| D-41 | M | Phase-scoped entitlement grant set (`phase→grant-set map`) | **DECIDED (autonomous) 2026-07-12 · flagged for veto** |
| D-42 | H | PriceOverlay: single adjustment → per-plan adjustment lines (reopens F-88) | **PROPOSED 2026-07-13 · flagged for veto** |
| D-43 | M | Prepaid grant: `category` + `applicability` + Billing-owned drawdown order (Stripe parity) | **DECIDED (autonomous) 2026-07-14 · flagged for veto** |
| D-44 | H | Level-based billing in launch: authorable `aggregationFunction {sum, peak, time_weighted}` + `aggregationGranularity` (supersedes F-40 "not at launch") | **DECIDED (product call) 2026-07-16 · CONFIRMED 2026-07-16** |
| D-45 | M | First-class `includedAllowance {quantity, rolloverPolicy}` in launch, publish-compiled (`none` → $0 band + marker; `carry` → D-43 grant) — supersedes F-32 | **DECIDED (product call) 2026-07-16 · CONFIRMED** |
| D-46 | M | Registry `sellable` flag on SKU (products gear) + sellability-gate predicate 6 (standalone lines; components exempt) | **DECIDED (product call) 2026-07-16 · CONFIRMED** |
| R-01…R-14 | — | Ratify the fix wave's applied decisions (section G) | **ALL CONFIRMED 2026-07-10** |

## A. Design-lock blockers

#### D-01 [H] S4 tax-completeness check has no data source

- **Where**: [`design/04-currency-tax.md`](./design/04-currency-tax.md) C4 + `TaxDisplayValidator` + §6; PRD §17.4 "Tax-inclusive" row.
- **Problem**: C4 blocks publish on "`taxInclusive=true` without a region tax rate" and "`taxInclusive=false` in a region with no configured `taxCategory`" — but no table/config anywhere stores per-region tax rates or per-region `taxCategory` (Tax Engine, which owns rates, is post-MVP). The validator's key predicate has no input. Worse, under the default fail-closed policy the flagship AC ("mixed plan publishes with `not_sellable_ga` on the EU rows") is unreachable — at MVP *every* region lacks a rate, so the row would block, not flag.
- **Options**: (a) add a per-region tax config (rate-present marker + `taxCategory`) to the region-taxonomy rows (or a `pricing_policy_object` entry), name it as the C4 input, and state the ordering — C4 completeness evaluates against tenant config; a *passing* `taxInclusive=true` row still publishes flagged `not_sellable_ga` (C3); (b) drop C4's rate predicate at MVP entirely (keep only `taxCategory` completeness), reinstate when Tax Engine lands.
- **Recommendation**: (a) — keeps the PRD §17.4 predicate honest and makes the AC constructible.
- **Decision**: **(a), 2026-07-10** — as the `RegionTaxReadiness` **port**: `(tenant, region) → { taxCategory, ratePresent }`, fail-closed on unknown. MVP provider = tenant-declared columns `tax_category` + `tax_rate_present` on `pricing_region_taxonomy` (CatalogAdmin, `config × write`, audited; catches configuration mistakes — rate correctness is unverifiable pre-Tax-Engine). Post-GA provider = Tax Engine-backed (sync vs event-fed mirror + the contract land in the Tax Engine PRD); tenant markers are then reconciled — divergence flags published rows + `pricing.tax.readiness_divergent` (Warn), remediation = re-publish. Ordering: C4 readiness gate precedes the C3 `not_sellable_ga` flag. **Propagated**: S4 C4/§1.7/`algo-tax-display` (`inst-td-readiness`)/§6/§9/§10; PRD fr-tax-display-basis + §17.4 (both rows).

#### D-02 [H] Second grandfathering cutover on the same scope key is impossible

- **Where**: [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) `inst-co-copy`/`inst-co-bounds`, W2; Foundation §4.3; PRD §17.5.
- **Problem**: the copy always lands on the single `existing_grandfathered` eligibility value. A second cutover on the same key (annual reprice, each cohort keeps its price) must schedule copy #2 overlapping copy #1's open-ended window → non-overlap violation, and every escape hatch (supersede the copy, shorten its window under `grandfatherUntil`) is explicitly forbidden. The design supports exactly one grandfathered generation per key and never says so; there is no error code for the rejected second cutover.
- **Options**: (a) make the limit normative — "at most one grandfathered generation per canonical scope key"; a cutover on a key whose grandfathered sibling holds an active/scheduled window is rejected (`CUTOVER_GRANDFATHERED_OCCUPIED`, 409) unless that row's `grandfatherUntil` is at/before the new cutover instant; (b) add a generation discriminator to the eligibility axis (multi-generation cohorts) — needs an ADR + PRD scope-key change.
- **Recommendation**: (a) for launch; (b) is a real product capability question (stacked legacy cohorts) — take it only with a product mandate.
- **Decision**: **(b) in full, laid in now — 2026-07-10** (product call: multi-cohort retention is table stakes; no N=1 launch boundary). **ADR-0002** (`cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`): the canonical scope key gains the additive `cohort` axis = the cutover instant (`none` on non-grandfathered rows; publish-enforced `cohort ≠ none ⇔ existing_grandfathered`). Every cutover creates a **new coexisting generation**; prior generations untouched. Within the grandfathered class, Tariffs selects the row by the **cohort of the subscription's pinned price id** (`pricingSnapshotRef` — no new binding store); most-specific-wins stays class-only. Default-move renewal semantics preserved (the Stripe-model analysis: full per-subscription binding was rejected for inverting the renewal default; industry mapping in the ADR). Note: D-04 (window bound at `grandfatherUntil`) and D-05 (retirement vs pending cutover) now apply **per generation**. **Propagated**: ADR-0002 (new) + ADR-0001 amendment note; Foundation §1.2/§3.7/§4.1/§5; DESIGN §1.2/§2.2/§3.6/§5; PRD glossary (Price row, priceEligibility, Grandfathering, grandfatherUntil, new `cohort` entry), §2.2, fr-grandfathering-eligibility, fr-supersession, AC #8/#71 key tuples, §17.4 duplicate-scope, §17.5 cutover row, §13 Tariffs row; S7 W2/W3, cutover flow/algo (`inst-co-copy`/`inst-co-atomic`/`inst-co-single-pending`), eligibility (`inst-el-fields`/`inst-el-generation`), state machine, DoD, ACs.

#### D-03 [H] Cutover atomicity across the external window SoR

- **Where**: [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) `inst-gc-commit`/`inst-co-atomic`, W1 (windows live in the effective-dating UC; the slice holds an event-driven mirror).
- **Problem**: the design claims "shorten + two schedules commit as one transaction", but the three operations execute in another component via the UC contract — a local ACID transaction cannot span them. Partial failure (shorten committed, copy-schedule failed) opens exactly the coverage gap the atomic unit exists to prevent; no compensation, no atomic multi-op UC API, no re-validation at commit is specified.
- **Options**: (a) require the UC contract to expose an **atomic batch window operation** (shorten+schedule+schedule accepted/rejected as one) — cross-team ask; (b) define a saga: schedules first, shorten last, explicit compensation on failure, plus a gap-freeness re-check against fresh UC state inside the commit step.
- **Recommendation**: (a) if the UC team accepts (cleanest, and other batch users exist — D-28); otherwise (b) written out normatively.
- **Decision**: **(c) — consolidation, 2026-07-10.** Investigation of the source UC (`vhp-architecture/docs/bss/prd/PRD-product-catalog-marketplace-202601120119/UC-effective-dating-price-windows-202601121200.md`) showed: it targets the defunct monolithic "Catalog Service"; its content is 100% price windows (no other consumers — registry unmentioned); its normative deltas were already superseded by this PRD; the atomicity requirement was already assigned to it via the consolidate-UC open question. Given the one-gear topology (all slices = one deployable, one PostgreSQL; Tariffs separate), **window ownership moves into the pricing gear**: S7 owns the `pricing_price_window` store + state machine (`scheduled/active/expired/cancelled`), the scheduling/cancellation API, the coordination-lease activation/expiration job, and `PriceWindow*` event production (frozen manifest names). The cutover's multi-window unit becomes a **local ACID transaction** — D-03 dissolves; **D-26 dissolves** (no UC-side mutations exist; mirror + `mirror_lag`/`coverage_gap` alarms removed); D-05 simplifies (single transactional domain). Legacy-UC dispositions: FX rate-lock rejected (no FX in catalog); impact preview out of scope (needs Subscriptions data); `suspended` state not adopted. PRD §15 consolidation question **answered** (formal Architecture ack pending). **Propagated**: S7 (§1, W1, names, flow, `inst-fg-when`, new `state-price-window` + window API/codes/table/events/alarms/DoD/ACs/§10), S11 (retirement invokes S7 flow; table refs), S12 §10 (bulk window ops local), S5 (window-endpoint authz rows), Foundation (§1.2 events, §3.4/§3.5, §4.3), DESIGN/README, PRD (12 spots incl. §2.1 boundary, §9.2 contract internalized, §13 row, §15 answer), legacy UC banner (vhp-architecture). **Formalized as ADR-0003** (`cpt-cf-bss-pricing-adr-pricewindow-consolidation`).

#### D-04 [H] Coverage hole between `grandfatherUntil` expiry and renewal re-bind

- **Where**: [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) `inst-co-bounds`/`inst-el-expiry`; PRD fr-grandfathering-eligibility.
- **Problem**: the copy's window must cover only **through** `grandfatherUntil`, but re-bind happens at the **next renewal** — a subscription renewing N days after expiry is bound to a key with no window for N days; usage/arrears rating on that key fails closed for a legitimate subscriber. Alternatively, if the intent is that an expired subscription immediately eligibility-matches the successor, its price changes mid-cycle before any re-bind — stated nowhere.
- **Options**: (a) the copy's window MUST cover `grandfatherUntil` **plus the longest billing cycle sold on that key** (tighten `inst-co-bounds` + PRD); (b) after expiry, Tariffs eligibility-matching resolves the subscription to the successor row even before the Subscriptions re-bind (re-bind is bookkeeping) — mid-cycle price change, needs product sign-off.
- **Recommendation**: (a) — no mid-cycle surprises, purely catalog-side rule.
- **Decision**: **(a), 2026-07-10** — a generation's window MUST cover `grandfatherUntil` + the **longest billing cycle sold on that key** (open-ended when null); enforced at cutover and on every `effectiveTo` adjustment. The margin keeps every bound period rateable until its renewal re-bind; it leaks nothing (new subscriptions never bind grandfathered rows). Post-D-02 the old cost (key occupancy delaying the next cutover) is gone — generations coexist. **Propagated**: S7 `inst-co-bounds` + key constraints + unit AC; PRD fr-grandfathering-eligibility.

#### D-05 [H] Retirement cancels the scheduled halves of an approved cutover

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-rt-cancel` ("cancel every not-yet-active window; active windows run to their natural end") vs [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) cutover flow.
- **Problem**: after an approved-not-yet-effective cutover, the state is: current window shortened to the cutover instant (active), copy + successor scheduled (not yet active). Retirement cancels both scheduled windows, and "natural end" of the active window was already moved to the cutover instant — in-flight subscribers lose all coverage at that instant. The future-gap check can't see it (it detects gaps *between* windows, not a trailing void).
- **Options**: (a) extend the retirement-triggered review with a **trailing-coverage check**: cancelling a scheduled window is rejected while an active window on the same/sibling eligibility key was shortened by an approved cutover whose instant hasn't passed; (b) retirement **unwinds** the pending cutover (restores the original `effectiveTo`, voids the cutover unit) as part of its approval unit.
- **Recommendation**: (b) — retirement expresses clear intent; unwinding is more operator-predictable than a rejection the operator can't easily interpret. Cross-reference from S11 `inst-rt-cancel` and S7 either way.
- **Decision**: **(b) + always-material, 2026-07-10** — retirement **unwinds** a live cutover unit inside the retirement transaction (one ACID scope post-D-03): predecessor window `effectiveTo` restored to its recorded pre-cutover value, scheduled copy/successor cancelled, unit closed as `unwound` (a merely `submitted` unit is voided per pin semantics); dry-run lists the unwind; prior generations untouched. Retirement with a live cutover is registered **always-material** in the S5 evaluator. **Propagated**: S7 `inst-co-retirement-unwind` (new step 7 of `algo-cutover`); S11 `inst-rt-api`/`inst-rt-cancel` + integration AC; S5 `inst-mat-registered` + threshold DoD; PRD AC #35 (two new And-clauses).

#### D-06 [H] Overlay/membership read-model propagation has no publish trigger

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) §7 ("overlays/memberships project into the read model **with the next relevant publish/warm**") vs `inst-pl-return` (immediate projection), Foundation §4.4 (read model monotonic per `CatalogVersion`), PRD AC #56.
- **Problem**: PriceOverlays and memberships have no publish/versioning trigger of their own, but the read model Tariffs pins only advances on a plan publish. An authored overlay/membership becomes rateable at an arbitrary later time (possibly never in a quiet tenant); a payer enrolled today can renew tomorrow against a pinned version that predates the membership — freezing the wrong (no) group. Contradicts publish-through-engine.
- **Options**: (a) overlays/memberships **publish through the Foundation engine** (own validation pass + `CatalogVersion` addressability request + warm — AC #56's "list is published" reading); (b) declare them a **separately versioned side-channel** with its own monotonic version + propagation SLO that Tariffs pins alongside `CatalogVersion`, and state which version the frozen snapshot records.
- **Recommendation**: (a) — one propagation mechanism, one determinism story; the registry batching already amortizes version churn.
- **Decision**: **(a), 2026-07-10** — every committed `PriceOverlay`/membership mutation is its **own publish unit through the Foundation engine** (validation → pending `CatalogVersion` ref → warm); consumer visibility is version-pinned exactly like plan content, so a renewal after the commit always sees the membership (the freezing race dissolves). No dedicated event (consumers observe `CatalogVersionPublished` + warmed content); registry batching coalesces bulk-enrollment churn; the overlay/membership trigger line joins the open §15 increment-taxonomy item with Registry. **Propagated**: S9 `inst-pl-return`/`inst-gm-return`/§7 + both DoDs + new integration AC; Foundation §4.2 (publish units are not only plans); PRD fr-priceoverlay-authoring + fr-customer-group-pricing + §17.5 increment table (new row) + §14 registry note.

#### D-07 [H] Rev-share: exact shares vs tolerant shares

- **Where**: [`design/08-bundles.md`](./design/08-bundles.md) `inst-rs-residual`/§6/AC ("shares must be exact", `RESIDUAL_UNASSIGNED`) vs PRD AC #23 ("default absorber = the platform… e.g. 33.33%×3 within ≤ 0.01% tolerance").
- **Problem**: three contradictions in one mechanism: (1) the PRD's own worked example (33.33×3 = 9999 bp) fails the design's exact-sum publish check; (2) the PRD default-absorber (platform) makes the design's `RESIDUAL_UNASSIGNED` unreachable; (3) the platform-as-absorber is unrepresentable in the data model (absorber flag lives on vendor rows; the platform cut is a bare column).
- **Options**: (a) **exact-shares** — amend PRD AC #23 (drop the 33.33×3 authoring example and authoring-time tolerance; tolerance applies to downstream *monetary* rounding only), keep or drop the platform default (if kept: add a platform party row or `residual_absorber = platform` sentinel and delete `RESIDUAL_UNASSIGNED`); (b) **tolerant shares** — relax the design to |Σ−10000| ≤ 1 bp with the nominated absorber taking the delta.
- **Recommendation**: (b) — matches operator reality (percentages come from contracts as 33.33%) and the PRD's intent; keep the absorber explicit-or-platform-default.
- **Decision**: **(b) with publish-time normalization, 2026-07-10** — authoring accepts `|Σ − 10000| ≤ 1 bp` (= the PRD's 0.01%); publish **normalizes** the bundle-level `residual_absorber` (a vendor SKU or the `platform` sentinel; default platform — an unnominated state cannot exist) so published **effective shares sum to exactly 10000 bp**; typed values retained for audit, adjustment recorded; over-tolerance → `RESIDUAL_OVER_TOLERANCE` (422) — a 6-way even split (9996 bp) must be operator-reconciled. Downstream (Tariffs/Marketplace) reads only effective shares — determinism intact; monetary (cent) rounding at settlement stays a separate downstream rule on the same absorber. `RESIDUAL_UNASSIGNED` deleted; `REVSHARE_UNBALANCED` narrowed to structural malformation. Data model: `share_bp` (typed) + `effective_share_bp` (published) on `pricing_bundle_revshare`; absorber moves to `pricing_bundle.residual_absorber`. **Propagated**: S8 (§1.1, B2, `RevShareReconciler`, flow errors, `inst-rs-residual`, §5 codes, §6 tables + constraints, Rev-Share DoD, AC); PRD AC #23 + fr-bundle-composition.

#### D-08 [H] `fixed`/amount PriceOverlay adjustments have no currency axis

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) `inst-plv-adjustment` + `pricing_price_overlay` columns; PRD fr-priceoverlay-authoring (also silent).
- **Problem**: an overlay's scope (partner/brand/customerGroup/global) spans base rows in many currencies, but `adjustment_value` is a bare minor-unit integer. A `fixed` (or absolute discount/markup) overlay is ambiguous the moment its target sells in ≥ 2 currencies: applying a EUR amount to a USD row is forbidden FX; failing closed on every non-matching market makes the overlay near-useless. Precision validation ("at the currency's minor unit") can't even run without a currency.
- **Options**: (a) amount-based adjustments carry per-`(currency)` (or per-`(currency, region)`) values — a child table like `pricing_grant_price`; (b) an amount-based adjustment MUST declare a single currency and its target coverage is restricted to base rows of that currency (publish-time check + error code).
- **Recommendation**: (a) — mirrors how price rows and grant prices already work; (b) as launch-scope fallback if authoring cost matters. Backfill PRD §6.6 either way.
- **Decision**: **(a), 2026-07-10** — money exists only per-currency, explicitly authored (the same discipline as base rows and grant prices). Percent adjustments stay a single bp value (currency-neutral); amount-based adjustments carry a **`pricing_price_overlay_amount`** value set (`UNIQUE (price_overlay_id, currency)`, minor-unit-validated) that MUST cover **every currency the overlay's target scope sells** — missing currency fails authoring (`ADJUSTMENT_CURRENCY_NOT_COVERED`, 422). Drift (a base row in a new currency published later): flag `coverage_incomplete` + `pricing.priceoverlay.amount_coverage_incomplete` (Warn); the uncovered market resolves **without** the overlay (normal precedence semantics — base price), remediation = add the value. (b) rejected: N same-class overlays would collide on precedence uniqueness. **Propagated**: S9 `inst-plv-adjustment`, §5 code, §6 child table + `adjustment_value` narrowed to bp, §7 alarm, PriceOverlay DoD, unit + integration ACs; PRD fr-priceoverlay-authoring, AC #26 enumeration, AC #55.

#### D-09 [H] Payer in multiple customer groups: resolution undefined

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) `inst-cg-resolve`/`inst-cg-freeze` + `pricing_group_membership` (non-overlap is per `(payer, group)` only); PRD fr-customer-group-pricing / AC #109 (singular "the resolved group").
- **Problem**: a payer may hold `trial` and `vip` simultaneously (overlap is rejected only within one group), yet freezing and the tie-break assume exactly one resolved group — two `customerGroup` overlays are the *same* class, so `inst-plv-class-tiebreak` cannot order them. Which adjustment applies and which group freezes is nondeterministic — a money outcome.
- **Options**: (a) enforce **at most one active membership per payer across all groups** (widen the non-overlap constraint; reject cross-group overlap at enrollment); (b) plural freezing (`resolvedGroups[]`) + a deterministic selection/stacking rule Tariffs adopts (e.g. per-group `precedence`, one overlay wins; document in PRD §17.7).
- **Recommendation**: (a) for launch — smallest surface, deterministic by construction; groups-as-segments rarely need stacking (discount stacking belongs to Promotions).
- **Decision**: **(a), 2026-07-10** — at most **one active membership per payer across all groups**, enforced at write; a conflicting enrollment → `MEMBERSHIP_CONFLICT` (409, names the active membership); a transfer is the atomic **move** operation (`POST …/members/{payerId}/move` — end current + start new at one instant, audited, standard materiality rules). "The resolved group" stays a truthful singular everywhere; in the catalog, groups exist only for pricing overlays, so two concurrent segments = two concurrent money rules — exactly the ambiguity removed; multi-membership + winner/stacking rule recorded in §17.8 Future (revisit with Promotions). **Propagated**: S9 `inst-cg-resolve`, `inst-ms-move` (state machine), move API row, `MEMBERSHIP_CONFLICT` code, membership-table constraint, customer-group DoD, unit AC; PRD fr-customer-group-pricing, §17.7 Membership row, AC #109 And-clause, §17.8 Future row.

## B. Governance

#### D-10 [M] Threshold-policy mutation is itself ungoverned

- **Where**: [`design/05-governance.md`](./design/05-governance.md) `PUT /v1/pricing/config/approval-threshold-policy` (ETag only; FinanceReviewer).
- **Problem**: the slice's own threat model ("a config admin must not weaken its own approval thresholds") stops at CatalogAdmin — but a **lone FinanceReviewer** can raise all thresholds to ∞ with one audited PUT, after which an accomplice's changes auto-publish approver-free. The most leverage-bearing governance object is the one mutation exempt from the two-person mechanism. (The fix wave already pinned: materiality is evaluated once at submit — a policy change never re-evaluates pending approvals.)
- **Options**: (a) route `approval_policy` PUTs through the approval workflow (always material; approver ≠ the policy submitter); (b) accept the risk explicitly: an alarm (`pricing.governance.threshold_weakened`, Warn, fired on any threshold increase) + a normative risk note.
- **Recommendation**: (a) — it is exactly the two-person rule's home turf; the workflow already exists.
- **Decision**: **(a), 2026-07-10** — the threshold-policy PUT opens an **always-material approval unit** (registered trigger): the diff applies only after an **independent** second FinanceReviewer approves; **direction-agnostic** (any policy diff — no fragile "was it weakening?" computation on multi-currency diffs); standard pin/void semantics apply; in-flight submissions keep their submit-time materiality (already pinned). Bootstrap fail-safe: a single-reviewer tenant leaves the policy unset ⇒ everything material. **Propagated**: S5 `inst-mat-registered`, AuthZ-catalog SoD note, §5 API row, Threshold DoD, new integration AC; PRD fr-approval-threshold-policy + §17.6 row.

#### D-11 [M] Bundle publish authz: no role can execute the stated conjunction

- **Where**: [`design/05-governance.md`](./design/05-governance.md) endpoint mapping ("publish of a bundle = `bundle × write` + `plan × publish`") + role matrix (FinanceManager: `plan × publish` but only `bundle × read`; ProductManager: `bundle × write` but no `plan × publish`) vs [`design/08-bundles.md`](./design/08-bundles.md) (names finance-manager/catalog-admin as the publishing actors).
- **Problem**: under the conjunction only CatalogAdmin can publish a bundle; S8 promises FinanceManager can. One of the three statements is wrong.
- **Options**: (a) grant FinanceManager `bundle × write`; (b) publish endpoint requires `plan × publish` only (composition was already authored under `bundle × write`); (c) introduce `bundle × publish` as its own action.
- **Recommendation**: (b) — publish is a plan-level act; authoring authority was already exercised and pinned by the approval hash.
- **Decision**: **(b), 2026-07-10** — the bundle publish endpoint requires **`plan × publish` only**; authoring stays `bundle × write` (ProductManager/CatalogAdmin); the composition is protected at publish time by the approval content pin, and component checks are validations, not caller authz. Role matrix unchanged; S8's actor claim (FinanceManager publishes) becomes true. **Propagated**: S5 endpoint mapping (row split), S8 flow actor line.

#### D-12 [M] Finance access to history/audit reads vs the role matrix

- **Where**: [`design/05-governance.md`](./design/05-governance.md) `inst-au-read` ("Auditor/Finance filters") + [`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md) `HistoryExporter` — vs the matrix (`audit × read/export`: Auditor **only**).
- **Problem**: two docs promise Finance a surface the normative matrix denies; implementers will quietly widen the grant or Finance is locked out.
- **Options**: (a) add `audit × read` to FinanceReviewer (and optionally FinanceManager); (b) change both docs to "Auditor filters" only — Finance uses the price-history read (`plan × read`) instead, and `/v1/pricing/history` splits from `/v1/pricing/audit`.
- **Recommendation**: (b) if price history alone satisfies the Finance use case (it usually does — audit rows carry actor PII discipline); else (a) for FinanceReviewer only.
- **Decision**: **(b) — surface split, 2026-07-10.** The mismatch was a modeling error: price **history** (chronological view over append-only price rows — effective dates, amounts) is plan/price data, not audit data. Remapped: `GET /v1/pricing/history` + `POST /v1/pricing/history/export` → **`plan × read`** (Finance in by construction, nothing new granted); `GET /v1/pricing/audit` (actor trails, before/after, approval decisions) stays **`audit × read/export`, Auditor-only**. Role matrix unchanged; PII boundary intact. **Propagated**: S5 endpoint mapping (rows split) + `inst-au-read`; S12 API table (2 rows), actor table, History & Export DoD, §10 security line.

#### D-13 [M] Historical-import governance: pipeline + second person

- **Where**: [`design/05-governance.md`](./design/05-governance.md) backdating flow vs [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-sy-backdate` (synthesis is the sanctioned consumer).
- **Problem**: the import path checks grant + reason + "zero downstream billable effect" — it never states whether imported rows run the Foundation validation pipeline (taxonomy, precision, scope-key duplicates), and requires no approval (one `BackdateGrant` holder). Yet backdated rows shape `migrated-origin` snapshots that rating consumes going forward — the "fraud-adjacent" path is weaker-governed than an ordinary price edit.
- **Options**: (a) imports run the same fail-closed pipeline (minus window scheduling) **and** rows reachable by snapshot synthesis require an approval record; (b) pipeline yes, approval no — provenance + audit is declared the accepted compensating control (with Finance sign-off recorded here).
- **Recommendation**: (a); the volume is low and the blast radius is billing history.
- **Decision**: **(a) — both controls, 2026-07-10.** (1) **Row-shape pipeline**: every imported row runs the fail-closed subset — taxonomies, ISO precision, scope-key uniqueness, model shape (window-coverage/sellability/addressability don't apply to reference rows, which resolve only via synthesis provenance); an import can never create a row regular authoring would reject. (2) **Two-person on all imports** (not "synthesis-reachable only" — reachability is undecidable upfront): historical import registered as an **always-material** trigger; `BackdateGrant` holder submits, independent FinanceReviewer approves, rows land on approval (202 → completion); import volume is migration-era-low, blast radius is billing history. **Propagated**: S5 backdating flow (`inst-bd-pipeline`/`inst-bd-twoperson`, error scenarios, return step), always-material trigger list, Backdating DoD, 2 ACs; PRD fr-historical-import-governance + AC #65; S11 `inst-sy-backdate`.

#### D-14 [L] G4 WORM vs hash chain: the transactional-audit claim

- **Where**: [`design/05-governance.md`](./design/05-governance.md) G4 + §10 ("audit writes share the mutation transaction").
- **Problem**: an external WORM store cannot join the DB transaction — the crash-consistency claim silently forces the hash-chain arm; and nothing says a mutation fails closed when the audit substrate is down.
- **Options**: (a) commit to hash-chained audit rows in the same DB (ledger precedent) — claim holds as written; (b) keep WORM as an option but require write-ahead to the transactional log, asynchronously anchored to WORM, and state: an unavailable audit substrate fails the mutation closed.
- **Recommendation**: (a); it was already the default candidate.
- **Decision**: **(a), 2026-07-10** — **in-DB hash-chained audit rows**, committed inside the mutation's ACID transaction: the §10 crash-consistency claim becomes literally true, and "audit substrate down ⇒ fail closed" dissolves by construction (the audit table *is* the database). Periodic chain-verification job (the `pricing_audit_chain_verified` metric already existed); chain head MAY be **asynchronously anchored** to external WORM/object-lock storage as hardening — never on the mutation path (anchor cadence = implementation knob). **Propagated**: S5 G4, §1.5, `inst-au-tamper`, Audit DoD, §10 risks; PRD fr-audit-completeness + retention NFR + AC (3 "WORM or hash-chained" spots).

## C. Plan & price shape (S2/S3)

#### D-15 [H] Phase→price coverage rule missing

- **Where**: [`design/02-plan-definition.md`](./design/02-plan-definition.md) `PhaseGraph` (`inst-ph-map`…); Slice 7 `CoverageChecker` (row-based, blind to phases); PRD AC #26 required-coverage set.
- **Problem**: a trial→intro→evergreen plan whose `intro` phase has **zero price rows** publishes cleanly (no rows ⇒ no windows ⇒ invisible to every check), then at phase conversion Tariffs resolves nothing and rating fails closed in production — exactly the "sold but unrateable" state S2 §1.2 promises impossible.
- **Decide**: (1) the rule — every phase id MUST be referenced by ≥ 1 published price row in every `(currency, region)` the plan sells (error code, e.g. `PHASE_UNCOVERED`); (2) the **hybrid semantics** — per phase, are both recurring and usage parts required, or are usage rows phase-invariant (one usage row spans phases)?
- **Recommendation**: adopt (1); for (2) declare usage rows **phase-invariant by default** (a usage row with `phase = evergreen` covers all phases unless a phase-scoped usage row exists) — matches how trials usually meter.
- **Decision**: **both parts, 2026-07-10** — (1) every phase id MUST be covered by ≥ 1 published **recurring** row per sold `(currency, region)`; violation → `PHASE_UNCOVERED` (422) in the S2 `PhaseGraph` (`inst-ph-coverage` — the row-based S7 coverage check cannot see a row-less phase). (2) **Usage rows are phase-invariant by default, phase-specific wins**: one usage row covers all phases; an explicit phase-scoped usage row overrides for its phase (a published resolution rule of the most-specific-wins class, adopted verbatim by Tariffs, joint fixture; free trial usage = explicit trial-phase row at 0 — never a silent default). **Propagated**: S2 `inst-ph-coverage`/`inst-ph-usage-invariant` + `PHASE_UNCOVERED` + dod-phases + integration AC; PRD fr-plan-phases, AC #54 (+2 clauses), AC #26 enumeration, glossary "Plan phase". Note: the exact axis value of "the default phase" formalizes in D-19.

#### D-16 [M] Add-on dependency/conflict edges have no data model

- **Where**: [`design/02-plan-definition.md`](./design/02-plan-definition.md) `inst-cmp-addons` ("conflicting pairs and dependency cycles fail publish") vs `pricing_plan_addon_rule` (no depends-on/conflicts-with columns).
- **Problem**: the graph cycle/conflict check has no edges to walk — either the columns are missing or the edges are registry-owned and the consumed contract is unnamed.
- **Options**: (a) plan-authored edges: `depends_on_addon_sku_id[]` / `conflicts_with_addon_sku_id[]` (or a `pricing_plan_addon_edge` table); (b) compatibility/dependency edges are registry SKU metadata — name that consumed contract in §1.8 and validate against it.
- **Recommendation**: (b) if the registry already models SKU compatibility (check with registry team); otherwise (a).
- **Decision**: **(a), 2026-07-10** — edges are **plan-authored**: `depends_on_addon_sku_id[]` / `conflicts_with_addon_sku_id[]` on `pricing_plan_addon_rule`, values restricted to the same plan's add-on set (outside-set edge fails), conflicts normalized symmetric, cycle walk over `depends_on`, two required conflicting add-ons fail. Rationale: everything else in that table is already per-plan; no new registry capability blocks launch; registry-intrinsic SKU compatibility (if it ever appears) becomes an **additional** validation input — additive, no migration (noted in S2 §10). **Propagated**: S2 `inst-cmp-addons`, §6 columns, unit AC, §10 note; PRD fr-addon-rules.

#### D-17 [M] Who sets `fail_closed_top`

- **Where**: [`design/03-price-structure.md`](./design/03-price-structure.md) `inst-tb-top` ("publish persists the explicit maximum + the marker") vs `CLOSED_TOP_UNMARKED` (422) in §5.
- **Problem**: if the system stamps the marker, the error is unreachable; if the author must supply it as an acknowledgment, the instruction never says so. Also unstated: the marker is invalid on non-top bands.
- **Options**: (a) author MUST set `fail_closed_top = true` on a closed top band (explicit acknowledgment; absence → `CLOSED_TOP_UNMARKED`; publish still emits the advisory warning); (b) system-stamped; delete `CLOSED_TOP_UNMARKED`.
- **Recommendation**: (a) — a closed top is a rating-behavior choice the author should consciously own.
- **Decision**: **(c) — closed tops are forbidden entirely, 2026-07-10** (a sharper option raised in review): the top band MUST be open (`toQty = null`); a closed top fails publish (`TIER_TOP_CLOSED`, 422). Rationale: "price undefined above X" is never the commercial intent — capping usage is an entitlement **quota** (grant set, Subscriptions enforces), a money cap is the per-period fee cap (already Tariffs Future, §17.8), and a different price above X is just another band. Consequences: "sold but unrateable" impossible by construction on tiered rows; the `fail_closed_top` marker, `CLOSED_TOP_UNMARKED`, the publish warning, and the Tariffs/Rating fail-closed-above-max evaluation branch are all **deleted**. Honest loss: a leaked quota now bills overflow at the top-band rate instead of raising loud exceptions — acceptable at launch (the compliance-grade tool is the per-period money cap, Future). Reintroduction path (author-acknowledged marker, the old option (a)) recorded in §17.8. **Propagated**: S3 Q1/§1.2/§1.5/§1.7/`inst-tb-top`/§5 code/§6 column removed/DoD/AC; PRD fr-tier-validation, AC #12, §5.2 flow, §17.4 contiguity row, §17.8 row; DESIGN driver row.

#### D-18 [M] Tiered non-usage rows: legal or forbidden

- **Where**: [`design/03-price-structure.md`](./design/03-price-structure.md) `inst-mk-forbidden` (eval-policy fields fail only on `flat` non-usage / `per_unit` rows); PRD §17.1 vs §17.4.
- **Problem**: a `graduated` row with `chargeKind = recurring` falls into no rule: nothing forbids it, nothing defines its `Q` (meaningless without a meter), nothing requires/forbids `tierAggregationWindow` on it.
- **Options**: (a) at launch `graduated`/`volume` are valid on `chargeKind = usage` rows only — `MODEL_KIND_CHARGEKIND_MISMATCH` (422); (b) define tiered-recurring semantics (quantity source = seats? no aggregation window) and extend `inst-mk-forbidden`.
- **Recommendation**: (a) — seat-tiered pricing is representable later as `per_unit` + bands, a Future item; don't invent semantics now.
- **Decision**: **(a), 2026-07-10** — `graduated`/`volume`/`package` are valid **only on `chargeKind = usage`** (`MODEL_KIND_CHARGEKIND_MISMATCH`, 422): the tier machinery presupposes a metered quantity stream; no `Q` semantics exist for non-usage rows. Tiered per-seat (bands over seat count on recurring) recorded in §17.8 Future (requires extending `quantitySource` to banded kinds). Side note ratified in discussion: the two tier-price selection semantics are already first-class — `graduated` = marginal per-band, `volume` = the total-quantity band's rate on all units (Tariffs Variant A only), both fixture-gated. **Propagated**: S3 `inst-mk-chargekind` + §5 code + unit AC; PRD fr-model-kind, §17.6 Model kind row, AC #26 enumeration, §17.8 Future row.

#### D-19 [M] `phase` scope-key axis typing

- **Where**: [`design/01-foundation.md`](./design/01-foundation.md) §4.1 (`phase = evergreen`, a reserved literal) vs [`design/02-plan-definition.md`](./design/02-plan-definition.md) `pricing_plan_phase.phase_id` (uuid, "referenced by the `phase` scope-key axis").
- **Problem**: the axis is a union of one reserved literal and per-plan uuids; whether a phased plan's *terminal* phase rows carry its uuid or the literal is undefined — and duplicate-scope, coverage, and supersession comparisons all hinge on it.
- **Options**: (a) the axis is always a `phase_id`; a well-known reserved `evergreen` phase row is auto-created per plan (the default is that id); (b) the axis value is the phase's kind-qualified slug (`evergreen`, `trial-1`, …) — human-readable, stable across revisions.
- **Recommendation**: (a) — uniform typing, no slug-collision rules; state it in Foundation §4.1 and align S2.
- **Decision**: **(a) refined, 2026-07-10** — the `phase` axis is **always a `phase_id`**; the default axis value = **the plan's terminal `phase_id`**. A phased plan uses its authored terminal phase (no second reserved entity); a non-phased/one-time plan gets **one implicit terminal phase row** (kind `evergreen`) auto-created at plan creation — setup rows ride the same id. The literal `evergreen` survives only as the phase *kind*. D-15's "phase-invariant" usage row = the terminal-phase row (resolution: phase-specific wins, else terminal). Clone copies phase rows with new `phase_id`s and remaps copied rows' axis. **Propagated**: Foundation §3.2 `ScopeKey` + §4.1; PRD §2.2; DESIGN §2.2; ADR-0001 defaults line; S2 `inst-ph-default`/`inst-ph-usage-invariant` + §6 table preamble; S12 `inst-cl-copy`.

#### D-20 [M] `customEveryN Months(n)` anchor semantics

- **Where**: [`design/02-plan-definition.md`](./design/02-plan-definition.md) P2/`inst-cs-customfreq` (constrains custom-**days** only); [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) K2 (month-end rule exists only for `fixed_day`).
- **Problem**: for `customEveryN Months(n)` nothing states the allowed anchors, and a `subscription_start` anchor on Jan 31 with monthly-equivalent cycles has no defined Feb date.
- **Proposal**: "`customEveryN Months(n)` MAY anchor `subscription_start` or `calendar_month`; a `subscription_start` anchor beyond the target month's length rolls to the last day of the month (same rule as `fixed_day`, K2); all UTC" — **confirm with Subscriptions** (they execute the math).
- **Decision**: **adopted + no-drift rule, 2026-07-10** — months cycles MAY anchor `subscription_start` or `calendar_month`; the K2 month-end clamp extends to `subscription_start` anchors, and the **anchor day is preserved** across periods (independent per-period clamp: Jan 31 → Feb 28 → Mar 31 — industry-standard no-drift semantics); UTC; rides the joint proration/anchor fixture with Subscriptions. **Propagated**: S2 P2 + `inst-cs-customfreq`; S6 K2 + `inst-pi-anchor`; PRD fr-custom-frequency, AC #45, glossary `billingAnchorPolicy`.

#### D-21 [L] Save-time vs publish-time validation split

- **Where**: [`design/03-price-structure.md`](./design/03-price-structure.md) authoring flow (`MODEL_KIND_MISSING` at POST, "full validation defers to publish") vs PRD AC #12 ("**save**/publish MUST fail" for band overlap/gap/ordering).
- **Decide**: which checks run at save (per-field shape: explicit kind on a banded row, precision, scope-key duplicate) vs publish (aggregate: contiguity, fixtures, coverage) — and reconcile AC #12's save-time expectation.
- **Recommendation**: band ordering/overlap/zero-width at **save** (they are row-local and cheap), contiguity/top-band policy at publish; state the split in S3 §2.
- **Decision**: **decided (autonomous), 2026-07-10** — sharper than the recommendation: **all row-local checks run at save and re-run at publish** (kind shape + kind×chargeKind matrix, full band-set geometry incl. contiguity — bands belong to one row, nothing about them is "aggregate" — precision, eval-policy placement, scope-key duplication); **aggregate/cross-entity checks are publish-only** (fixtures, window/phase coverage, hybrid completeness, injectivity). AC #12's save-time expectation satisfied. **Propagated**: S3 `inst-pr-return`.

#### D-22 [L] What the supersession-continuity fixture gates

- **Where**: [`design/03-price-structure.md`](./design/03-price-structure.md) fixture registry (the fix wave added `variant = supersession_continuity` with a proposal).
- **Decide**: ratify the proposal — the continuity fixture gates the **first publish of any tiered usage kind** (alongside that kind's own fixture) — or scope it differently (e.g. gate the first *supersession* instead of the first publish).
- **Decision**: **ratified as proposed (autonomous), 2026-07-10** — gating the first *publish* (not the first supersession) is fail-safe: by the time a supersession happens the fixture must already have proven `Q`-continuity, and it keeps one uniform FixtureGate trigger. **Propagated**: S3 §6 registry note.

## D. Consumer contracts & windows (S6/S7)

#### D-23 [M] Rule-based `allowedChangeTargets` defeat publish-time guarantees

- **Where**: [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) `inst-pc-targets`/`inst-pc-mutual`; PRD fr-plan-change-contract.
- **Problem**: "dangling target fails publish" and "every target carries a `comparabilityRank`" are only checkable over an explicit list; for a rule, targets exist at read time — a rule can resolve to an unpublished or rank-less plan, making A→B classification uncomputable with no specified failure mode.
- **Options**: (a) restrict rules to tenants with an authoritative published `PlanTier` ordering (rank always derivable); (b) specify read-time fail-safe semantics: an unresolvable/rank-less rule-resolved target is excluded from the resolved set and the contract is marked `partially_resolvable` in the read model; (c) drop rules at launch (explicit lists only).
- **Recommendation**: (c) at launch, (b) as the designed extension — rules without enforcement teeth are drift generators.
- **Decision**: **(c) + (b)-as-Future (autonomous), 2026-07-10** — launch = **explicit published `planId`s only**; rule-based targets not authorable (a rule resolves only at read time, defeating every publish-time guarantee). The designed extension (read-time fail-safe resolution + `partially_resolvable` marker) recorded in §17.8. *Product-flavored: narrows a PRD-listed option — flagged for veto.* **Propagated**: S6 `inst-pc-targets`; PRD fr-plan-change-contract, glossary, AC #108, §17.8 row.

#### D-24 [M] Retired target left in the change graph

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-re-references` (guards bundles + add-on overrides only); [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) `inst-pc-targets`.
- **Problem**: after target B retires, plan A still publishes edge A→B; whether Subscriptions may execute a self-service change into a retired plan is undefined (a plan change is not clearly a "purchase" for the sellability gate).
- **Proposal**: (1) add `allowedChangeTargets` referrers to S11's retire dry-run enumeration (as **warn**, not block — a change edge is softer than a bundle reference); (2) one sentence in S6: an edge whose target is later retired is inert — Subscriptions MUST re-check the target's lifecycle state at change time.
- **Decision**: **warn + inert-edge (autonomous), 2026-07-10** — retire dry-run enumerates `allowedChangeTargets` referrers as a **warning** (a change edge is softer than a bundle reference); the edge goes **inert** and Subscriptions re-checks the target's lifecycle state at change time. **Propagated**: S11 `inst-re-references`; S6 `inst-pc-targets`; PRD fr-plan-change-contract.

#### D-25 [M] Cross-boundary change edges neither rejected nor marked

- **Where**: [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) K3/`inst-pi-crossboundary` vs `inst-pc-targets` (no boundary check); contrast S11 where the same mismatch is a blocking migration delta. The commonest self-service upgrade (monthly→annual) crosses frequency.
- **Options**: (a) publish-time classification of each edge as `in_place` vs `cancel_plus_new` (published on the edge; storefront can disclose credit forfeiture); (b) reject `cancel_plus_new` edges at publish until the K3 cross-team sign-off lands.
- **Recommendation**: (a) — the sign-off (PRD §15) needs the classification anyway; publishing it is the catalog's half of the contract.
- **Decision**: **(a) (autonomous), 2026-07-10** — publish classifies every edge `in_place` vs `cancel_plus_new` and publishes it on the edge; Subscriptions/storefront disclose credit forfeiture before execution; re-computed on either side's re-publish. **Propagated**: S6 `inst-pc-boundary`; PRD fr-plan-change-contract, glossary, AC #108.

#### D-26 [M] Must UC-side window mutations route through Slice 7

- **Where**: [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) `inst-fg-when` — the fix wave added mirror-event re-checks + the `pricing.window.coverage_gap` Critical alarm (detection). Routing is the open half.
- **Decision**: **RESOLVED by D-03 (2026-07-10)** — windows are gear-owned; no UC-side mutation path exists. Every window mutation goes through `WindowScheduler`/`CutoverOrchestrator` with in-transaction coverage validation; the mirror, `mirror_lag`, and `coverage_gap` alarms are removed (the window tables carry the same REVOKE + column-whitelist trigger discipline as `pricing_price`).

#### D-27 [M] Resolved grant-set drift after a registry tier-policy change

- **Where**: [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) `inst-gs-resolved` (frozen resolved set) — the registry can change a `PlanTier`'s feature/quota policy after publish; the tier-*value* drift got a flag+alarm (S2 `inst-cmp-tier-drift`), the tier-resolved *grants* did not.
- **Proposal**: mirror the S2 mechanism — consume the registry tier-policy-change signal, flag affected published plans `grants_divergent` (+ alarm), remediation = re-publish (re-resolving the set); consumers keep the frozen set meanwhile. Needs the registry signal to carry policy-level changes (joint-contract scope, PRD §15).
- **Decision**: **adopted as proposed (autonomous), 2026-07-10.** **Propagated**: S6 `inst-gs-drift` + `pricing.contracts.grants_divergent` alarm; registry-signal scope noted for the §15 joint contract.

#### D-28 [L] Multi-key (batch) cutover shape

- **Where**: [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) (idempotency per `(planId, scope key, instant)`); [`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md) (mass repricing excludes grandfathered rows and creates supersessions, not cutovers).
- **Problem**: "raise prices everywhere, grandfather everyone at T" decomposes into one cutover + one two-person approval per `(currency, region, phase, chargeKind)` key, with no cross-key same-instant consistency guarantee.
- **Options**: (a) a multi-key cutover payload in S7 (one approval unit spanning the plan's keys at one instant); (b) an S12 "mass cutover" run type; (c) accept N units at launch.
- **Recommendation**: (a) — it is the natural extension of the existing unit and reuses the S5 per-row hash pin.
- **Decision**: **(a) (autonomous), 2026-07-10** — the cutover payload carries a **scope-key selector**: all selected keys cut over at one instant as **one approval unit / one local ACID transaction** (post-D-03), per-key generations created, every touched key pended, S5 per-row hash pin covers the set; idempotency per `(planId, key-set hash, instant)`. **Propagated**: S7 API row; PRD §17.5 cutover row.

## E. Bundles, price overlays, primitives (S8/S9/S10)

#### D-29 [M] Prepaid GA-gate mechanics

- **Where**: [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) `inst-pg-gagate` ("the same flag mechanism as Slice 4's `not_sellable_ga`") — but S4's derivation input (`taxInclusive` on the row) and granularity (per market) don't transfer to a plan-attached grant.
- **Decide**: (1) the deriving input — a named platform/tenant GA signal "prepaid balance execution GA" (owner: Billing/Rating, tracked on the program board per PRD §13); (2) the scope — the flag applies to **every scope key of the grant-bearing plan** (matches the PRD AC #87 plan-level clarification); (3) clearing — like S4: re-publish through the pipeline once the signal is GA, never a silent flip (+ a one-line state machine in S10 §4).
- **Recommendation**: adopt (1)–(3) as written.
- **Decision**: **adopted (autonomous), 2026-07-10** — (1) derives from the named platform/tenant signal "prepaid balance execution GA" (owner Billing/Rating, program board); (2) plan-level: every scope key of the grant-bearing plan (matches AC #87); (3) clearing = re-publish through pipeline + approval (S4 `inst-td-clear` pattern), never a silent flip. **Propagated**: S10 `inst-pg-gagate`.

#### D-30 [M] `ResolvedGroupFreezer` ownership

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) §1.7/`inst-gm-return` — a catalog component "freezes the resolved group into snapshots", but the resolved group is per-payer, resolved at activation/renewal by Tariffs, and the composition SoR is Tariffs; the catalog has no per-subscription snapshot participation and no resolve-for-payer API.
- **Options**: (a) re-scope `ResolvedGroupFreezer` as the **joint contract definition**: catalog publishes membership into the read model; Tariffs performs interval resolution and freezes the group into the snapshot it composes (adjust §1.7, `inst-gm-return`, DoD); (b) add a catalog resolution endpoint Tariffs calls (new API + latency contract in §5).
- **Recommendation**: (a) — keeps the composition SoR where every other doc puts it.
- **Decision**: **(a) (autonomous), 2026-07-10** — `ResolvedGroupFreezer` re-scoped as the **joint contract name**: the catalog publishes membership into the read model; **Tariffs** resolves at activation/renewal and freezes the group into the snapshot **it composes** (composition SoR); no catalog resolve-for-payer endpoint. **Propagated**: S9 §1.7 naming row + `inst-gm-return`.

#### D-31 [M] Retiring a plan targeted by a `PriceOverlay`

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-re-references` (bundles + add-on overrides only); [`design/09-price-overlays.md`](./design/09-price-overlays.md) `inst-plv-referential` (authoring-time only).
- **Problem**: the retire path silently re-creates the forbidden dangling-overlay state; an overlay targeting a retired plan sits in the read model with undefined Tariffs behavior and no alarm (contrast `pricing.discount.ref_dangling`).
- **Options**: (a) add overlay `target_ref`s to the retire guard (`RETIRE_PLAN_REFERENCED`) — blocks retirement until overlays are ended; (b) dangle-and-remediate: Warn alarm `pricing.priceoverlay.target_retired` + read-model flag, mirroring the discountRef pattern (in-flight subscribers keep resolving retired plans anyway).
- **Recommendation**: (b) — an overlay is an adjustment on top of rows that legitimately outlive retirement for in-flight subscribers; blocking is disproportionate. State the choice in S9 §10 + S11.
- **Decision**: **(b) dangle-and-remediate (autonomous), 2026-07-10** — read-model flag + `pricing.priceoverlay.target_retired` (Warn), overlay stays evaluable for in-flight subscribers; retire dry-run enumerates targeting overlays as a warning; remediation = end/retarget. **Propagated**: S9 `inst-plv-referential` + §7 alarm; S11 `inst-re-references`.

#### D-32 [L] Composite-meter output-unit ownership

- **Where**: [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) (persists `output_unit` as a bare string); PRD Glossary (registry declares base units; silent on derived units).
- **Options**: (a) the derived unit is declared to the registry like any `meteringUnit` (Rating recognizes it via the registry); (b) it is a catalog-namespaced id published in the read model only.
- **Recommendation**: (a) — one meter namespace; avoids a parallel identity scheme downstream.
- **Decision**: **(a) (autonomous), 2026-07-10** — the derived output unit is **registry-declared like any `meteringUnit`** (Rating recognizes it via the same registry lookup); the catalog persists the registry unit id + the formula binding; part of the registry joint contract (§15). **Propagated**: S10 `inst-cm-output-unit`; PRD glossary (derived meter).

#### D-33 [L] F-34 member-scoped preview as a tracked GA gate

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) `inst-plv-member-preview` ("REQUIRED for storefront UX") vs PRD §15 F-34 ("Open. Owner: Tariffs + GTM").
- **Decide**: either register F-34 as a program-board GA gate (owner + target date; the S9 wording becomes "required **before** restricted segment pricing goes self-service — GA-gated on F-34"), or drive F-34 to Answered now.
- **Recommendation**: register the GA gate; don't hold the slice on it.
- **Decision**: **GA gate registered (autonomous), 2026-07-10** — F-34 stays Open with Tariffs+GTM but is now a **tracked program-board GA gate**: restricted segment pricing does not sell self-service until it lands; nothing else holds. **Propagated**: S9 `inst-plv-member-preview`; PRD §15 F-34 row.

## F. Lifecycle & operator efficiency (S11/S12)

#### D-34 [H] Cancelling an `in_progress` migration

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) transition 2's parenthetical ("cancellation of `in_progress` only stops further processing") vs the state machine (no `in_progress → cancelled`), the DELETE contract (before-effective only), `MIGRATION_ALREADY_EFFECTIVE` (409), M3, and the AC.
- **Problem**: mid-batch stop of a partially-executed migration is the risky operational case; the doc half-promises it in a parenthetical while every normative surface forbids it. (Left as-is by the fix wave — this is the decision.)
- **Options**: (a) no cancel once `in_progress` — delete the parenthetical; an operator's only lever is the completion report + a follow-up migration back; (b) add `in_progress → cancelled` (stops further `PlanLink` processing; already-migrated unaffected; partial set listed on the record) and rescope DELETE/`MIGRATION_ALREADY_EFFECTIVE` to `completed` runs only.
- **Recommendation**: (b) — a stop-the-bleeding control on a fan-out that can span thousands of subscriptions is worth the extra transition; pairs with D-38.
- **Decision**: **(b) (autonomous), 2026-07-10** — `in_progress → cancelled` added (halts further `PlanLink` processing; already-migrated unaffected; partial sets listed); only `completed` is uncancellable; DELETE rescoped; `MIGRATION_ALREADY_EFFECTIVE` → `MIGRATION_COMPLETED` (409). *Operational-control flavored — flagged for veto.* **Propagated**: S11 state machine (`inst-mst-cancel-inflight`), API row, §5 code, AC; PRD fr-scheduled-migration.

#### D-35 [H] Bulk/repricing rows vs pending approval units on one key

- **Where**: PRD fr-supersession ("at most one pending approval unit per canonical scope key"); [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) `inst-co-single-pending`; [`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md) (bulk import + mass repricing create per-key supersessions under one batch approval — the interplay is unspecified).
- **Decide**: (1) a batch row whose scope key already holds a pending interactive unit — fails Phase-1 validation (or is a listed per-row conflict)? (2) does a **submitted batch approval count as the pending unit** for each contained key (interactive submits on those keys 409 naming the bulk operation, mirroring the bulk lock)?
- **Recommendation**: yes to both: batch row → per-row conflict at Phase-1 naming the pending unit; a submitted batch pins its keys (409 for interactive submits) — symmetric, and the per-row hash pin already gives the batch that identity.
- **Decision**: **both (autonomous), 2026-07-10** — Phase-1 per-row failure naming the pending interactive unit (import + repricing selectors alike, `inst-mp-pending`); a `submitted` material batch **pins every contained key** (interactive submit → 409 `PENDING_CHANGE_UNIT_EXISTS` naming the bulk op). **Propagated**: S12 `inst-bk-phase1`/`inst-bk-approval-subset`/`inst-mp-pending`; PRD AC #112.

#### D-36 [M] Execution-time re-validation of locks + boundary deltas

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-cl-source` (locks resolve "at validation time"), `delta_report` ("at schedule time"); M5 default notice 60–90 days.
- **Problem**: between scheduling and `effective_at`, a new contract can lock an in-scope subscription (it would be migrated — breaking "a lock is never broken"), and the target's coverage for a frozen `(currency, region)` can lapse. The strongest guarantee is enforced at a point that can be months stale.
- **Options**: (a) on `scheduled → in_progress` the catalog re-resolves the lock set and boundary deltas — newly-locked subscriptions are excluded (appended to the completion record); a newly-broken boundary delta fails that subscription's `PlanLink` closed into the exception list; (b) assign the execution-time re-check to Subscriptions as part of the `PlanLink` joint contract (§10 open item).
- **Recommendation**: (a) for locks + boundary (catalog owns both inputs), with the per-subscription enforcement handshake documented in the joint contract either way.
- **Decision**: **(a) (autonomous), 2026-07-10** — on `scheduled → in_progress` the catalog re-resolves the lock set (newly-locked excluded, appended to the record — a lock is never broken however stale the schedule) and the boundary deltas (a target that lost coverage fails that subscription's `PlanLink` closed into the exception list). **Propagated**: S11 `inst-mst-start` + `inst-cl-source` + AC; PRD fr-scheduled-migration.

#### D-37 [M] Bulk-lock crash/timeout release path

- **Where**: [`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md) — the lock releases only via state-machine completion; a crash mid-`committing` freezes interactive authoring on the marked rows indefinitely (only a Warn alarm exists).
- **Proposal**: (1) the bulk runner holds a **coordination lease** (the library DESIGN.md §3.8 already names); on lease takeover the successor re-drives Phase-2 from the journal/report; (2) an operator **abort** transition `committing → completed_with_conflicts` (uncommitted rows reported as not-attempted; lock cleared).
- **Decide**: confirm lease-resume + abort; or abort-only (simpler, loses auto-resume).
- **Decision**: **lease-resume + abort (autonomous), 2026-07-10** — the runner holds a coordination lease (takeover re-drives Phase 2 from the journal, idempotent); operator `:abort` transitions `committing → completed_with_conflicts` (uncommitted rows `not-attempted`, lock cleared). A crashed import can never freeze authoring indefinitely. **Propagated**: S12 §6 bulk-lock note + `inst-bs-abort`.

#### D-38 [L] Migration-cancellation propagation to Subscriptions

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-mg-cancel` (read-model state + audit; no event) — Subscriptions creates `PlanLink`s on the scheduled event (push) but learns of cancellation only by re-reading (pull); the T-ε race (cancel accepted moments before execution starts) is unhandled.
- **Options**: (a) pull obligation in the §10 joint contract: Subscriptions MUST re-read the schedule state immediately before beginning execution; the catalog's cancel is rejected (`MIGRATION_ALREADY_EFFECTIVE`) once Subscriptions has reported execution start — the state handshake, not the wall clock, is the arbiter; (b) introduce a cancellation event (amends the frozen event set — §7 currently forbids it).
- **Recommendation**: (a) — no event-set change; the handshake also serves D-34(b).
- **Decision**: **(a) (autonomous), 2026-07-10** — a **state handshake, not a wall clock**: Subscriptions re-reads the schedule state before beginning execution and per processing batch thereafter; it never starts/continues against a cancelled record (closes the T-ε race and serves D-34's in-flight cancel); no new event name. **Propagated**: S11 `inst-mg-cancel`.

#### D-39 [M] Migration entry phase on a phased target (trial re-entry)

- **Where**: [`design/11-lifecycle.md`](./design/11-lifecycle.md) `inst-mg-boundary` (covers the setup row only); S2 phase machinery is silent on `PlanLink` entry.
- **Problem**: a migrated subscription entering a target plan's `trial`/`intro` phase gets a fresh free/discounted period — the same revenue-leak class the setup never-re-charge clause closed; PRD is silent, so Subscriptions would improvise.
- **Options**: (a) normative: a migrated subscription enters the target's **first non-trial phase** (a migration never grants a new `trial`; whether it may enter `intro` — sub-decision); carried on the `PlanMigrationScheduled` contract; (b) surface targets-with-trial-phases as an informational migration delta and leave the entry rule to a Product decision.
- **Recommendation**: (a) with "first non-trial phase" (i.e. `intro` allowed, `trial` never); mirror one clause into PRD fr-scheduled-migration.
- **Decision**: **(a) (autonomous), 2026-07-10** — a migrated subscription enters the target's **first non-trial phase** (a migration never grants a new `trial`; `intro` allowed); the rule rides the `PlanMigrationScheduled` contract. *Revenue-policy flavored — flagged for veto.* **Propagated**: S11 `inst-mg-boundary` + AC; PRD fr-scheduled-migration.

#### D-40 [M] Trailing-tier qualification vs the two existing tier windows

- **Where**: [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) `cpt-cf-bss-pricing-algo-trailing-tier`; [`design/03-price-structure.md`](./design/03-price-structure.md) Q2 (`tierAggregationWindow` resets the in-window counter); PRD §Advanced Pricing Primitives.
- **Problem**: a real PaaS-traffic pattern needs the **rate tier chosen from the *prior* month's total** while the current month is **billed hourly on actual usage** — the rate is known at period start and applied forward. `tierAggregationWindow` only expresses *current-window* accumulation (climb tiers within the period) and `billingGranularity` only sets billing cadence; neither expresses "prior-period qualifies, current-period billed", so the model can't represent it.
- **Options**: (a) overload `tierAggregationWindow` with a `trailing_month` value (conflates *counter reset* with *tier qualification*); (b) add a **separate** `tierQualificationWindow` (`current` \| `trailing_month`) as a Slice-10 advanced primitive, keeping the three windows orthogonal (qualification vs in-window aggregation vs billing granularity); (c) model it downstream in Tariffs only (no catalog field).
- **Recommendation**: (b) — the three concerns are genuinely distinct; overloading one enum (a) makes valid combinations unexpressible, and (c) hides an authored commercial choice from the catalog/snapshot.
- **Decision**: **(b) (autonomous), 2026-07-12** — add **`tierQualificationWindow`** (`current` default \| `trailing_month`) on tiered usage rows. `trailing_month` qualifies the rate tier from the **prior period's total** (single-band **volume**-style selection), **locks that one rate** for the current period into `pricingSnapshotRef` (the tier analogue of the FX rate-lock), and bills actual usage at `billingGranularity`; `current` preserves existing behaviour exactly. Forbidden on non-tiered/non-usage rows (`TIER_QUAL_ON_NON_TIERED`, 422); first-period **bootstrap** resolves to the lowest tier (or an authored bootstrap) and freezes. Ownership split follows the tier pattern: catalog **authors** the window and freezes it; **Rating** computes the trailing aggregate and re-qualifies at each period boundary; **Tariffs** applies the locked rate — the catalog never computes the aggregate. *Revenue-policy flavored — flagged for veto.* **Propagated**: S10 `cpt-cf-bss-pricing-algo-trailing-tier` + `inst-tt-*` + §5 `TIER_QUAL_ON_NON_TIERED` + DoD `cpt-cf-bss-pricing-dod-trailing-tier`; PRD §6.10 Trailing-tier qualification primitive row.

#### D-41 [M] Phase-scoped entitlement grant set

- **Where**: [`PRD.md`](./PRD.md) `cpt-cf-bss-pricing-fr-entitlement-grant-set` + §Entitlement grant set glossary; [`design/01-foundation.md`](./design/01-foundation.md) `ReadModel` (`phase→price map`).
- **Problem**: a time-boxed trial that should confer **smaller quotas than the paid phase** (e.g. 20 cloudlets / 1 IP during a 14-day trial, then unlimited / 16 IP in evergreen) has no representation: the model publishes **per-phase price** (`phase→price map`) but the **entitlement grant set is plan-level** (`PlanTier`-driven, one set per plan). There is no `phase→grant-set map`, so trial and evergreen cannot differ on limits — only on price.
- **Options**: (a) keep the grant set plan-level; model trial limits some other way (a separate trial *plan* with its own PlanTier, converted-to via plan change) — no per-phase entitlements, extra plan + migration; (b) add an optional **`phase→grant-set map`** to the published read model (mirroring `phase→price map`): a phased plan MAY author a grant set per phase; absent per-phase entries the plan-level `PlanTier` grant set applies; Subscriptions resolves the grant set for the **active phase at `t`**; (c) make `PlanTier` itself phase-resolvable (larger blast radius — PlanTier drives tax/GL/commercial tier, not just entitlements).
- **Recommendation**: (b) — smallest additive change that mirrors the already-accepted `phase→price` shape; keeps `PlanTier` plan-level (a) avoids the extra-plan/migration overhead and matches how operators think ("one trial offer, tighter caps early").
- **Decision**: **(b) (autonomous), 2026-07-12** — the published read model MAY carry a **`phase→grant-set map`**: a phased plan authors an entitlement grant set (feature flags + quotas) **per phase**, published like `phase→price`; where a phase has no entry the **plan-level `PlanTier`-driven** grant set applies (backward-compatible). The catalog **publishes** the map and validates referential integrity (feature/quota/`PlanTier` defined in the registry — reuses the grant-set publish gate); **Subscriptions** resolves the grant set for the **active phase at `t`** and owns enforcement (this PRD never enforces). `PlanTier` stays plan-level (option (c) rejected). *Revenue/entitlement-policy flavored — flagged for veto.* **Propagated**: PRD `cpt-cf-bss-pricing-fr-entitlement-grant-set` + §Entitlement grant set glossary row; DESIGN `ReadModel` `phase→grant-set map`.

#### D-42 [H] PriceOverlay — single adjustment → per-plan adjustment lines (reopens F-88)

- **Where**: [`design/09-price-overlays.md`](./design/09-price-overlays.md) `inst-plv-adjustment` (one `adjustment_kind` + magnitude per list), §6 `pricing_price_overlay.adjustment_*`, §10 Risks ("adjustment-only overlay is the committed shape", F-88, 2026-07-04); PRD `cpt-cf-bss-pricing-fr-priceoverlay-authoring`.
- **Problem**: an overlay carries **one** adjustment (`markup|discount|fixed` + magnitude) applied across every plan its `target_ref` names — so a single negotiated segment deal that is **−20% on plan A, −15% on plan B, −10% on plan C** cannot live in one `PriceOverlay`. The committed answer is *N* separate same-class overlays (one per rate), a separate plan (different structure), or Contracts (per-account). But a `customerGroup` resolves such lists into one segment context, and authoring "one deal" as three sibling overlays with hand-managed precedence is the ergonomic gap operators hit in the studio.
- **Options**: (a) keep F-88 (single adjustment per list); differentiated per-plan pricing = multiple overlays (status quo — no model change); (b) make a `PriceOverlay` a **container of per-plan adjustment lines** — each line keyed `(planId, targetSku?)` with its own `adjustment_kind` + magnitude; **most-specific-wins within a list** (a `targetSku` line beats the whole-plan line for the priced row), **class rank still stacks across lists**; (c) full price-overlay-items matrix (per plan × currency × region × SKU) — the Zuora/SAP "price overlay = line items" model in full (largest surface).
- **Recommendation**: (b) — the smallest shape that closes the ergonomic gap while staying **adjustment-only** (it does *not* reopen "different tier structures", which stays Future / separate-plan per `inst-cg-routing`). It keeps the overlay-not-axis rule (L1) and the cross-class tie-break intact; it only refines precedence uniqueness (see consequences).
- **Consequences to resolve before adopting**: precedence uniqueness `UNIQUE (tenant_id, scope_class, precedence)` (L2) presumes one adjustment per list — with per-plan lines the collision domain narrows to **`(scope_class, planId, target)`**, or precedence is dropped *inside* a list in favour of the within-list most-specific rule (cross-list ordering stays class-rank). D-08 per-currency amount coverage (`pricing_price_overlay_amount`) and referential integrity (`inst-plv-referential`, D-31 dangling-on-retire) both **re-attach per line**. `inst-cg-routing` is **unchanged** (still: adjustment → overlay; structure → separate plan; per-account → Contracts).
- **Decision**: **PROPOSED 2026-07-13 — prototyped in Pricing Studio, flagged for veto.** The studio's `PriceOverlay` now holds `lines: [{ planId, targetSku?, adjKind, adjPct|fixedValue }]`; the price engine resolves the most-specific line per `(plan, sku)` and stacks lists by class rank; the editor is a per-`(plan × target)` line grid. This **reopens F-88's committed adjustment-only-*single*-magnitude shape** — a MAJOR launch-scope decision (2026-07-04) — so it does **not** land in the normative design until Product/Finance rule on it. *Commercial-model flavored — flagged for veto.* **Propagated (prototype only)**: Pricing Studio (`PRICE_LISTS[].lines`, `overlayLineFor`, per-plan line editor); this entry + veto markers on `inst-plv-adjustment` / §10 Risks and PRD fr-priceoverlay-authoring.

#### D-43 [M] Prepaid grant — definitional parity with a Stripe credit grant (category, applicability, drawdown order)

- **Where**: [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) `cpt-cf-bss-pricing-algo-prepaid-grant` + §5/§6/§8/§9; PRD `cpt-cf-bss-pricing-fr-prepaid-credit-grant`, §17.7 grant table, §17.4 "Prepaid grant" row, AC #90; source analysis: [STRIPE-GAP-ANALYSIS.md](./STRIPE-GAP-ANALYSIS.md) G-2.
- **Problem**: the grant carried only `grantAmount` / `creditUnit` / `expiryPolicy` / `autoRechargeAllowed` / `price`, so three **definitional** (authoring-time, not execution) capabilities were unrepresentable: (1) a **free promotional** grant — `price` was unconditionally required; (2) **spend scoping** ("this wallet applies to egress but not to compute") — structure authored at definition time, not balance state; (3) any stance on **drawdown order** when one account holds several grants (promo-before-paid is the canonical expectation). Stripe authors all three on the credit-grant object (`category`, `applicability_config`, `priority`). Rating-side check: the wallet grant is disjoint from step-6 `commitmentPools[]` (rating SEAMS M8), so nothing downstream models this either.
- **Options**: (a) keep the thin grant and let Billing add category/scoping when balance execution lands — Billing would end up owning *definitional* fields, outside the snapshot; (b) add the three as definitional catalog fields — `category` (`prepaid | promotional`), `applicability` (usage-line scope, **materialized at publish**), `drawdownPriority` (authored **default** only) — while the **effective** cross-grant order stays Billing-owned under a normative deterministic tie-break chain; (c) have the catalog also resolve execution order (violates "no balance state here").
- **Recommendation**: (b) — additive, keeps the authoring/execution split exact (definition frozen in the snapshot; execution over account state stays Billing's), and mirrors the coupons principle: policy from the frozen snapshot, the executor never infers.
- **Decision**: **(b) (autonomous), 2026-07-14** — `category ∈ {prepaid (default), promotional}`: `promotional` is issued **free** (price rows MUST be absent — `GRANT_PROMO_PRICE_FORBIDDEN`; `autoRechargeAllowed` MUST be false — `GRANT_PROMO_AUTORECHARGE`; `expiryPolicy = never` warns — `GRANT_PROMO_NO_EXPIRY`). `applicability` = `all_usage` (default) or a set of **published** meters that are usage lines of the plan (never `one_time_setup` or recurring rows — launch rule); a metered `creditUnit` bounds the set to that unit's meters; publish **materializes** the resolved set into `pricingSnapshotRef` — the executor never infers scope. `drawdownPriority` (optional int, lower first) is an authored **default rank**; the **effective** order across an account's grants is **Billing-owned**, normatively `drawdownPriority` → `promotional` before `prepaid` → earlier expiry → earlier issuance → `grantId` (deterministic total order). Grant-price / `category` / `applicability` / `drawdownPriority` changes route through the material-change policy. Where drawdown sits relative to discounts and tax stays the Billing joint-contract open (STRIPE-GAP-ANALYSIS G-4). *Commercial-model flavored — flagged for veto.* **Propagated**: S10 `inst-pg-category` / `inst-pg-applicability` / `inst-pg-priority` (new) + `inst-pg-price` / `inst-pg-material` (amended) + §5 problem values + §6 `prepaid_grant` jsonb & `pricing_grant_price` iff-`prepaid` + DoD + §9 ACs + §10 risks; PRD glossary row, `fr-prepaid-credit-grant`, §17.4 "Prepaid grant" row, §17.7 grant table, AC #90, AC #26 aggregate fail-closed list; STRIPE-GAP-ANALYSIS G-2 marked actioned.

#### D-44 [H] Level-based billing in launch — authorable `aggregationFunction {sum, peak, time_weighted}` (supersedes F-40)

- **Where**: PRD §1.4 "Tier aggregation window" glossary row; PRD §15 answered-questions F-40 row; PRD §17.8 Follow-on row (→ Scope); [`DESIGN.md`](./DESIGN.md) §4 deferred list; [`design/03-price-structure.md`](./design/03-price-structure.md) Q2. Rating side: rating `T-D-17`, rating PRD §6.5 `fr-level-aggregation`, rating design/03 §4.3, design/12–13 (gauge intake + granule fold).
- **Problem**: F-40 was answered "not at launch" (2026-07-04) — usage `Q` window-**sum** only. The launch product set now confirmed to bill on **levels**: cloudlet **peak-per-hour** (Jelastic-heritage pay-per-use) and storage **GB-month** (time-weighted occupancy). Without an authorable aggregation the only bridge is source-side folding (source emits pre-folded counter deltas) — billable but wrong long-term: the commercial rule (peak? cadence?) lives in the emitter instead of the catalog, raw levels never reach audit, and retro re-aggregation is impossible.
- **Options**: (a) keep sum-only + source-side folding bridge for launch; (b) authorable `aggregationFunction ∈ {sum, peak, time_weighted}` with a **granule-fold** design — non-`sum` meters consume gauge samples, fold per `aggregationGranularity ∈ {hour (default), day}` granule (peak → max sample; time_weighted → step-integral level×dt), and window `Q` = **Σ granule folds**; (c) full Stripe-parity set incl. `last`/`unique`.
- **Recommendation**: (b) — the granule-fold keeps window-`Q` **additive by construction**, so every downstream invariant survives unchanged: the M7 counter key and single-writer, supersession continuity, T-D-12 `bandOffsetQ` slice math, package/volume band math, delta-only corrections (a late sample re-folds only its granule → `Q` delta → standard re-materialization). `last`/`unique` have no launch product and stay Future.
- **Decision**: **(b) (product call), 2026-07-16** — `aggregationFunction ∈ {sum (default), peak, time_weighted}` + `aggregationGranularity ∈ {hour (default), day}`, authorable on usage price rows, both frozen in `pricingSnapshotRef`. Non-`sum` rows: the meter is level-shaped (collector `gauge` kind); the sample unit is the level unit (GB, cloudlet), the billable `Q` unit is level·granule-hours (GB·h, cloudlet·h) — the SKU declares the **billable** unit (same doctrine as the composite output unit); publish validates sample-unit ↔ level-unit. Sampling-gap policy: `hold_last` bounded by a declared `maxHold`, beyond which the level reads 0 **and** an operator signal raises (fail-visible, never guessed) — `maxHold` value is Design-level. Non-`sum` MUST NOT co-occur with composite meters at launch (inputs stay window-sum). Joint Rating fixture required before publish of any non-`sum` row (AC #60-style). *Product-scope reversal — confirmed by the product owner 2026-07-16.* **Propagated**: PRD glossary/§15/§17.8 + DESIGN §4 + design/03 Q2 (this side); rating T-D-17 + PRD §6.5/§17.1 + design/03 §4.3 + design/12/13 (rating side); SEAMS M10 + UC6 updated.

#### D-45 [M] First-class `includedAllowance` in launch — publish-compiled (supersedes F-32)

- **Where**: PRD §1.4 (tier-row tail + new "Included allowance" glossary row), §6.10 `fr-included-allowance`, §15 F-32 row (re-decided), §17.8 row (→ Scope), AC #90a; [`DESIGN.md`](./DESIGN.md) §4 deferred list; [`design/03-price-structure.md`](./design/03-price-structure.md) Q6; [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) D-45-consumer note.
- **Problem**: F-32 (2026-07-04) kept allowances as the `$0`-first-band workaround. Existing-SKU migration (2026-07-16) needs "N included, then rate" as a first-class fact: the band covers the math but not display ("includes N units") or reporting (included vs billed), and `rolloverPolicy` — cross-period state — is inexpressible in a stateless band. The floated stopgap (netting the allowance at the usage source / CT facade) violates the D-44 doctrine: the commercial rule must live in the catalog, sources emit raw usage.
- **Options**: (a) keep $0-band + facade-only labeling (no rollover, no reporting split); (b) first-class authored `includedAllowance` with **publish compilation** — `none` → $0 band + frozen marker, `carry` → D-43 per-period promotional grant; (c) a new rating-side allowance evaluator (new step machinery).
- **Recommendation**: (b) — first-class authoring with **zero new evaluation machinery**: band math and grant drawdown both already exist and are fixtured; the compiler is publish-time, deterministic, and frozen into the snapshot.
- **Decision**: **(b) (product call), 2026-07-16, confirmed** — `includedAllowance = {quantity N > 0 (billable units), rolloverPolicy {none (default), carry(maxPeriods ≥ 1)}}` on `usage` rows; publish compiles per the Glossary/FR rules; publish fails on non-`usage`, double-free ($0 band + allowance), non-`sum` rows (level-meter allowance Future — variable level·granule boundary), `quantity ≤ 0`; per-seat scaling = named Future gate. Rollover **execution** stays Billing-owned (D-43 drawdown order); rating is untouched by construction. **Propagated**: PRD glossary ×2 / FR / F-32 / §17.8 / AC #90a; DESIGN §4; design/03 Q6; design/10 note.

#### D-46 [M] Registry `sellable` flag + sellability-gate predicate 6

- **Where**: products gear PRD (`gears/bss/products/docs/PRD.md`): glossary "Sellable", `fr-sku-sellable`, mutability bucket iii, AC #2/#2a, provenance note; this PRD `fr-sellability-gate` (predicate 6 + component exemption); [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) sellability DoD.
- **Problem**: `published` means *referenceable*, not *offerable*. Migrated catalogs carry technical/component SKUs that must exist, meter, and compose without ever being sold standalone; the only gates were plan/market-level (five predicates, `not_sellable_ga`) — no SKU-level field. The need was parked as a "CF Design-phase decision"; post upstream-detach the registry PRD is ours, so the decision lands here.
- **Decision**: **(product call), 2026-07-16, confirmed** — SKU-level `sellable` (default `true`; `false` = composition/metering-only), material-but-mutable (bucket iii, governed change, frozen per `CatalogVersion`); enforced as sellability-gate predicate **(6)** for **standalone** lines; bundle-**component** references exempt (the component conjunction stays (1)–(5), (6) applies to the bundle SKU itself). **Propagated**: products PRD (glossary/FR/mutability/AC 2+2a/provenance); pricing `fr-sellability-gate` (five→six); design/07 DoD; rating SEAMS §I RG3 closed alongside (first-substantive-edit rule).

## G. Ratifications — decisions already applied by the review fix wave

Each item below was applied as an "obvious fix" but embeds a judgment call. **All confirmed
2026-07-10** (several extended by later D-decisions — noted per row); none reverted.

| # | Applied decision | Where applied | Status |
|---|------------------|---------------|--------|
| R-01 | The **five sellability predicates** (S7's set) are now enumerated normatively in the PRD (fr-sellability-gate, §9.2) | PRD | **CONFIRMED 2026-07-10** |
| R-02 | Cutover emits **`PriceCreated` ×2** (copy + successor) + window events; both rows pass the Foundation pipeline and the commit **requests `CatalogVersion` addressability** like a supersession publish | PRD §17.5, S7 | **CONFIRMED 2026-07-10 (extended by D-03: windows gear-owned, the unit is one local transaction)** |
| R-03 | AC #87's discount-path GA flag is **plan-level** (every scope key of the grant-bearing plan), distinct from the per-market tax flag | PRD | **CONFIRMED 2026-07-10 (D-29 formalized the mechanics)** |
| R-04 | Custom-interval cap provisional values: `customEveryNDays ≤ 366`, `customEveryNMonths ≤ 24`, tenant-configurable (ratify the numbers before Design lock) | PRD nfr-size-limits / AC #84/#104 / §15 | **CONFIRMED 2026-07-10 (values still ride the §14 ratification)** |
| R-05 | **Bundle nesting forbidden at launch** — a component `planId` must not be a `bundle`-type plan (`COMPONENT_IS_BUNDLE`; nesting = Future) | PRD fr-bundle-composition, S8 | **CONFIRMED 2026-07-10** |
| R-06 | **Clone drops `existing_grandfathered` + superseded rows** (lifecycle state, not configuration) | PRD fr-plan-clone, S12 | **CONFIRMED 2026-07-10 (cohort-generalized by D-02: all generations dropped)** |
| R-07 | `migration_id` is **client-supplied** in the create POST (create-retry idempotency, mirroring S12 `run_id`) | S11 | **CONFIRMED 2026-07-10** |
| R-08 | Repricing journal: `pending \| applied \| failed` (+ one-transaction rule row+outbox+journal; run complete when no `pending`; `failed` retryable only via a new run) | S12 | **CONFIRMED 2026-07-10 (D-35 added pending-unit per-row failures)** |
| R-09 | Bulk import: **approval before Phase-2** (`awaiting_approval` state); bulk lock takes effect on entry to `committing`; response is **202 + GET report** | S12 | **CONFIRMED 2026-07-10 (D-37 added lease-resume + abort)** |
| R-10 | `pricing.migration.blocked_deltas` gauge → **`pricing.migration.blocked_total` counter** (blocked schedules never persist) | S11 | **CONFIRMED 2026-07-10** |
| R-11 | Foundation-owned problem types (`DUPLICATE_SCOPE_KEY`, `STALE_VERSION`, `IDEMPOTENCY_PAYLOAD_MISMATCH`, validation envelope with **advisory `warnings[]`**, 202 pending) — slices reference, never redefine | S1 §3.3, S3 | **CONFIRMED 2026-07-10** |
| R-12 | Append-only enforcement = trigger **column whitelist** (state-machine `lifecycle_state` flips + monotonic `grandfather_until` tightening); partial `UNIQUE` = one **current** row per key, temporal non-overlap enforced by S7 validation (not the index; window ownership consolidated into S7 by D-03/ADR-0003); `pricing_plan` mutable-in-place under the state machine; **history in-table** (no `pricing_price_history`) | S1, ADR, DESIGN | **CONFIRMED 2026-07-10 (key now eight columns — ADR-0002; phase axis id-typed — D-19)** |
| R-13 | **Pending approval is not a lifecycle state** (subject stays `draft`, mutation voids the record); the publish commit **re-runs the validation pipeline** inside the commit transaction (approval approves content, commit re-validates state) | S1 §3.6/§4.2, S5 | **CONFIRMED 2026-07-10 (D-10/D-13 route policy + imports through the same workflow)** |
| R-14 | `EligibilityExpirySignal` is **derived at read time** (`now ≥ grandfatherUntil`), never stored, no job/event; the backlog alarm reads Subscriptions re-bind feedback (mechanism = §10 joint contract) | S7 | **CONFIRMED 2026-07-10 (per-generation since D-02)** |
