<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — manual slice review (2026-07-31)

**Scope**: all 12 slice designs ([`../design/`](../design/)) + [`DESIGN.md`](../DESIGN.md) +
[`DECISIONS.md`](../DECISIONS.md) (D-01…D-86, §F.1/§F.2) + targeted PRD verification.
**Method**: single-reviewer sequential pass, slice by slice, no subagents; every candidate
finding cross-checked against the decision register and the PRD before being claimed.
**Baseline**: everything already registered — open product forks (§F.1), carried findings
(§F.2), the D-79…D-86 wave and its 2026-07-31 veto confirmations — is deliberately **not**
re-reported. Only new findings below.

**Verdict**: the set is mature; five review waves have cleaned out most defect classes. What
this pass finds is one repeating pattern: **recent decisions landed on their cited surface but
not on the adjacent one** — D-76/D-81 made tier-2 synthesis reachable but nobody can consume
it; D-82 guarded supersession but not the phase axis; D-83 revisioned S2's child tables but
not S8/S9's; D-86 keyed the read model per plan while D-06's publish units have no plan; the
L-4 instant floor covered cutovers but not the everyday supersession — which itself had no
defined operation at all. Remaining: **3 [H]**, **9 [M]**, and a tail of [L].

> **Status (verification pass, same session): ALL FINDINGS CONFIRMED AND FIXED.** Every
> finding was independently re-verified against the cited document text during the review
> pass before fixing — none was rejected or downgraded (M-5 was folded into D-88 as the same
> mechanism; L-6 was escalated into D-98). H-1…H-3 and M-1…M-9 are closed as **D-87…D-98**
> in [`DECISIONS.md`](../DECISIONS.md); the [L] items are text fixes in place. The
> per-finding mapping is in the [Verification & fix record](#verification--fix-record).
> **D-93/D-94/D-98 are flagged for veto (D-93/D-94 joint with Subscriptions).**

Severity scale: **[H]** breaks money/correctness or is unimplementable as written · **[M]**
teams can build incompatible behavior · **[L]** contained.

---

## [H] Findings

### H-1. Tier-2 synthesis produces a snapshot nothing can consume — and tiered legacy prices cannot even be imported

D-76/D-81 made synthesis tier 2 *reachable*; its output was still unusable, twice over.
(i) [`05-governance.md`](../design/05-governance.md) §6 gave `pricing_historical_price` no
tier-band or package storage while the row-shape subset (`inst-bd-pipeline`) requires a
`graduated`/`volume` row to carry ≥ 1 band (S3 `inst-mk-required`) — a tiered legacy price
failed its own import validator with nowhere to put the bands. (ii) The synthesized
`migrated-origin` snapshot recorded resolved price **ids** — but a tier-2 id lives in a store
that is "never projected, never in a `CatalogVersion`, sole reader = synthesis"
([`01-foundation.md`](../design/01-foundation.md) §3.7), while rating must "retry against the
frozen result" ([`11-lifecycle.md`](../design/11-lifecycle.md) `inst-sy-firstrating`) and
charge from modelKind/bands/eval-policy it can read nowhere; which `CatalogVersion` a
`migrated-origin` ref pins was likewise unstated. The D-01 defect class: the rule is written,
the consumption mechanism does not exist. **Fix → D-87** (self-contained materialized payload;
band/package storage on reference rows; no version pin on `migrated-origin`).

### H-2. Supersession — the primary change mechanism — has no defined operation

The "supersession unit" is *named* throughout the set (one-pending-unit-per-key —
[`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md) `inst-co-single-pending`;
the D-35 key pins; the S5 approval hash) but no slice defines what composes one: no API, no
flow, no orchestrator for successor row + predecessor-window shorten + successor-window
schedule + `published → superseded` flip. Assembled from the documented primitives it is
unbuildable-or-broken — the successor's window collides with the predecessor's open-ended
window (`WINDOW_OVERLAP`); shortening first is its own always-material D-62 operation that
drops the key below the D-80 coverage horizon (a sales outage) until the successor schedules;
two approvals instead of one. The cutover got `CutoverOrchestrator`, `CUTOVER_GAP`, and the
D-47 instant floor; supersession got nothing — including no instant floor, so a changeover at
commit time activates the successor's window up to 5 minutes before the row is addressable in
any completed `CatalogVersion` (transient fail-closed renewals/arrears; the L-4 class, fixed
2026-07-30 for cutovers only — multiplied across every key of a repricing run). **Fix → D-88**
(the supersession unit: composition, API, one approval + one ACID commit, instant floor,
interactive and bulk alike — absorbing finding M-5).

### H-3. The D-82 ×24 class reopened through the phase axis

The tier counter is keyed `(subscription, meter, dimensionKey, window)` — **phase-blind**
([`03-price-structure.md`](../design/03-price-structure.md) `inst-tb-window-continuity`) — so
`Q` continues across a phase conversion; yet nothing constrained a phase-scoped usage
override's `billingGranularity`/`aggregationFunction`/`aggregationGranularity`/
`tierAggregationWindow`/`tierQualificationWindow`/`modelKind` against the terminal-phase row
it overrides ([`02-plan-definition.md`](../design/02-plan-definition.md)
`inst-ph-usage-invariant` imposed no field constraint). A `per_hour` trial row converting into
a `per_day` evergreen row mid-window applies an hours-denominated continued `Q` to
day-denominated bands — the D-77/D-82 ×24 band-edge class through a third door — and where the
window values differ the counter silently resets; neither continuation nor reset was stated.
**Fix → D-89** (phase-blindness stated normatively — a conversion never resets `Q`; the
override preserves the D-82/D-98 field list, `PHASE_OVERRIDE_UNIT_MISMATCH`; fixture scenario).

---

## [M] Findings

### M-1. "Current plan revision" is structurally ambiguous
`pricing_price` predecessors flip `published → superseded` at commit; plan **revision** rows
had no analogue ([`01-foundation.md`](../design/01-foundation.md) §3.7: states
`draft|published|retired`) — after revision N+1 publishes, revision N stays `published`
forever. No single-published constraint, no definition of "current", no stated retire target;
any truth-side `EXISTS(published)` query (overlay referential integrity, migration targets,
bundle components) could match a stale revision — the remembered-predicate hazard D-76 exists
to eliminate. **Fix → D-90** (flip-at-commit + partial `UNIQUE` one published revision).

### M-2. D-06 × D-86 never reconciled: overlay/membership publish units had no read-model representation
`pricing_read_model` was keyed `(tenant_id, catalog_version, plan_id)` (D-86) while D-06's
publish units are plan-less: a membership is per payer; a `global`/`brand`/`region` overlay
targets many or all plans — as per-plan deltas one global-overlay commit re-projects the whole
tenant, exactly the explosion D-86 forbids; "publishes membership into the read model"
([`09-price-overlays.md`](../design/09-price-overlays.md)) had no defined keying or surface.
**Fix → D-91** (subject-typed deltas `(tenant, version, subject_kind, subject_ref)`; an
overlay commit projects one overlay-subject row, never the targeted plans; membership rows
per payer record; same greatest-completed-≤-pin rule per subject — which also settles the
per-version-vs-per-row warm-marker ambiguity, finding L-1).

### M-3. D-83's revision discipline reached neither bundles nor overlays
`pricing_bundle_component`/`revshare`/`revshare_group` were keyed by `bundle_id` alone
([`08-bundles.md`](../design/08-bundles.md) §6); `pricing_price_overlay*` carried no
`lifecycle_state`/revision at all while the submit surface is idempotent "per revision"
([`09-price-overlays.md`](../design/09-price-overlays.md) §5/§6). A draft recomposition of a
published bundle, or a draft edit of a published overlay, mutates published truth; a
degraded-warm re-drive can leak the draft into a frozen version — the exact D-83 defect class,
two slices over. **Fix → D-92** (bundle child tables gain `plan_revision`; overlays get
draft-revision rows with flip-at-commit).

### M-4. D-25's stamped boundary classification cannot be re-computed under the frozen read model
`boundaryClass` was stamped into the source plan's `allowed_change_targets` at the source's
publish and promised to "re-compute on either side's re-publish"
([`06-consumer-contracts.md`](../design/06-consumer-contracts.md) `inst-pc-boundary`) — but a
target's publish unit warms only its own delta (D-86/D-91) and the source's published revision
is immutable: no mechanism exists. A stale `in_place` lets Subscriptions run an in-place
change across a currency/region/frequency boundary (wrong proration — money). **Fix → D-93**
(classification computed at change time by Subscriptions from both plans' published facts at
its pinned version — the same read-time discipline as `comparabilityRank`; the stamp removed;
flagged for veto, joint with Subscriptions).

### M-5. The changeover-instant floor covered cutovers only
`CUTOVER_INSTANT_PASSED` (the 2026-07-30 L-4 fix) had no analogue for supersessions or
repricing runs — the far more common path. **Folded into D-88** (`SUPERSESSION_INSTANT_PASSED`;
a repricing run names one changeover instant bounded against its approval commit).

### M-6. Sellability-gate granularity for multi-key plans was undefined
The gate was stated "for the bound canonical scope key" — singular
([`../PRD.md`](../PRD.md) `fr-sellability-gate`; S7 `inst-sg-surface`) — while a hybrid/phased
plan binds several keys; the conjunction was explicit only for bundles. The D-80 exemption
("not currently sellable") was therefore undecidable for component keys: an exempt cancel of a
usage-key window on a zero-subscriber hybrid whose recurring key stayed sellable reopened the
trailing void for the usage line (sold-but-unrateable). **Fix → D-94** (conjunction over every
key the purchase binds; the exemption evaluates plan-market sellability; flagged for veto,
joint with Subscriptions).

### M-7. Required-add-on coverage was checked per currency, not per `(currency, region)`
[`04-currency-tax.md`](../design/04-currency-tax.md) `inst-cb-addon` and PRD
`fr-invoice-currency-binding` (i) required "a row in a currency the base plan publishes" —
the region axis unchecked, while S2's override-home rule demands the full pair. A required
add-on covering EUR only in `US` while the base sells EUR in `EU` passed publish and died at
order assembly — the D-84 asymmetry one level up. **Fix → D-95** (per pair).

### M-8. `invoiceGroupingKey` had no design home
The PRD glossary defines it as an optional Plan field consumed by Billing; S4 references it
twice as existing; no slice persisted, validated, or projected it. **Fix → D-96**
(`pricing_plan.invoice_grouping_key`, revision-scoped, projected; layout hint only).

### M-9. An add-on override target's supersession left a stale frozen `priceId`
The override ref froze a specific `priceId` at the base plan's publish
([`02-plan-definition.md`](../design/02-plan-definition.md) `inst-cmp-override-home`); a
routine supersession of that row on the add-on's own plan left every referencing base plan
pointing at a superseded row whose window closes at the changeover — retire is guarded
(`RETIRE_PLAN_REFERENCED`), supersession was not, and successor inheritance was unstated.
**Fix → D-97** (the ref binds the row's canonical scope key; resolution follows the key
through windows, so the successor legitimately serves).

---

## [L] Findings

- **L-1.** Warm-completion marker ambiguity — §4.4 "a version is ignored until … the marker"
  (per version) vs the per-row marker of §3.7; partial-batch semantics unstated. **Fixed** in
  S1 §4.4 (per-subject marker; subjects resolve independently) — subsumed by D-91.
- **L-2.** `AVAILABLE_FROM_IN_PAST` fired on re-publish of a plan whose `availableFrom` had
  legitimately passed — any later revision was blocked until the date was erased. **Fixed**:
  the rule binds newly set/changed values only (S2 `inst-cs-availability`).
- **L-3.** Phase-graph shape underspecified: reachability/linearity unvalidated (a dead phase
  passed acyclicity + single-terminal and still demanded coverage rows); the entry phase —
  which D-39's "first non-trial phase" and setup timing depend on — was a convention.
  **Fixed**: linear chain over ordinals, entry = lowest ordinal, `PHASE_CHAIN_NONLINEAR`
  (S2 `inst-ph-graph`).
- **L-4.** Drift asymmetry: PlanTier and grant-set changes got operator flags; a registry
  metering-unit `usageTypeRef`/dimension-set change had no drift story. **Fixed**:
  `meter_binding_divergent` operator flag + Warn alarm (S2 `inst-cmp-usagetype`, S1 §3.7).
- **L-5.** A `submitted` approval unit pinned its scope key indefinitely with no escape but
  mutating the subject. **Fixed**: explicit `withdraw` (submitter/CatalogAdmin, audited) —
  S5 `inst-as-void` + endpoint.
- **L-6.** `model_kind` change across supersession was unguarded — a `graduated → volume`
  flip mid-window re-prices the accumulated window total under new math. **Escalated and
  fixed → D-98** (kind joins the D-82 preserved set; flagged for veto).
- **L-7.** Bundle coverage evaluates `all_subscriptions` rows only — a component priced solely
  via `new_subscriptions_only` can never be bundled. **Fixed**: stated as deliberate with the
  remediation named (S8 `inst-bc-coverage`).
- **L-8.** A `cohort` filter on the overlay **list-default line** had no defined validation
  (the check runs against "the line's target plan", which a default line lacks). **Fixed**:
  `CHECK (cohort IS NULL OR plan_id IS NOT NULL)` — per-plan lines are the way to target a
  generation across a scope (S9 §6 + `inst-plv-eligibility`).
- **L-9.** A `usage` floor inside the `[0, N)` allowance band produced no warning (the
  in-band warning fired on non-zero-priced bands only) while silently voiding part of the
  allowance. **Fixed**: warning extended (S10 `inst-ft-warn`).
- **L-10.** Audit hash-chain scope (per tenant vs global) was unstated — a cross-tenant chain
  is physically impossible under residency cells. **Fixed**: per-tenant chains (S5
  `inst-au-tamper`).

---

## Performance assessment

Strong overall (single indexed pinned reads; the order-time gate reads the pinned read model;
the repricing journal commits row+outbox+journal in one transaction; the activation job scans
by index). This wave's material points:

- **The M-2/D-91 gap was also the main perf risk**: implemented naively as per-plan deltas, a
  `global` overlay commit would have re-projected every plan of the tenant per mutation —
  the exact explosion D-86 forbids. The subject-typed answer writes **one row per publish
  unit**.
- **Repricing throughput**: the ratified ≥ 50 rows/s now explicitly includes the per-row
  supersession-unit window operations (2 window writes/row) inside the per-row transaction —
  already named in S12 §10; the perf test must exercise them (the O3 figure predates D-03's
  window consolidation).
- **D-79 lane on the mutating path**: synchronous, fail-closed, twice per retire (dry-run +
  commit) — bounded; a call-timeout budget is an implementation knob.

## Per-slice verdicts

| Slice | Verdict |
|-------|---------|
| 01-foundation | Core coherent; M-1/M-2 and L-1 rooted here |
| 02-plan-definition | H-3, M-8, M-9, L-2, L-3, L-4 |
| 03-price-structure | H-3 (co-owner), L-6; D-77/D-82 themselves landed accurately |
| 04-currency-tax | M-7; otherwise clean |
| 05-governance | H-1(a) — reference-row bands; L-5, L-10; still the most mature slice |
| 06-consumer-contracts | M-4; otherwise clean |
| 07-pricewindow-linkage | H-2, M-5, M-6; the window machinery itself is tight post D-62/D-63/D-80 |
| 08-bundles | M-3 (bundle half), L-7 |
| 09-price-overlays | M-2, M-3 (overlay half), L-8; D-42/D-78/D-67 landed accurately |
| 10-advanced-primitives | Clean (D-53/D-59/D-60 and the L-6-2026-07-30 fix all in place); L-9 minor |
| 11-lifecycle | H-1(b) — synthesis consumption; retirement/migration tight post D-51/D-65/D-79/D-80 |
| 12-operator-efficiency | M-5 (co-owner); otherwise consistent |

---

## Verification & fix record

Second pass, same session: every finding re-verified against the cited document text before
any fix was applied. **All 3 [H] + 9 [M] + 10 [L] CONFIRMED**; none rejected (M-5 merged into
D-88 as one mechanism; L-6 escalated to D-98).

Verification notes worth keeping: for **H-1**, the store spec's own words decided it — "a
reference row is field-complete or it is not importable" is unsatisfiable for a `graduated`
row without band storage, and "its only reader is snapshot synthesis" contradicts any rating
read, so the payload had to become self-contained rather than granting rating a second reader
(which would have reopened D-76's isolation argument). For **H-2**, the tell was
`inst-co-single-pending` naming "a pending approval unit (cutover **or supersession**)" — the
register legislated conflicts for an operation no document defines. For **M-4**, "re-computed
on either side's re-publish" was checked against D-86's delta semantics directly: the target's
publish unit cannot touch the source's rows, immutable by §4.3 — the promise was structurally
unkeepable, so the mechanism (not the wording) had to change.

| Finding | Verdict | Fix | Where it landed |
|---------|---------|-----|-----------------|
| H-1 (tier-2 synthesis not consumable; no reference bands) | **CONFIRMED** | **D-87** — self-contained materialized payload; `tier_bands`/package fields on `pricing_historical_price`; no version pin on `migrated-origin` | S5 §6 + `inst-bd-pipeline` + DoD; S11 `inst-sy-payload` (new) + `inst-sy-provenance` + §6 + DoD + AC; S1 §4.4; PRD `fr-migration-safety` + `fr-historical-import-governance` |
| H-2 + M-5 (no supersession unit; no instant floor) | **CONFIRMED** | **D-88** — the supersession unit (compose/approve/commit atomically) + `SUPERSESSION_INSTANT_PASSED` ≥ commit + max batching SLO, interactive and bulk | S7 `algo-supersession` + `SupersessionOrchestrator` + API + codes + DoD + AC; S3 `inst-ps-supersede`; S1 §4.3; S12 `inst-mr-api`/`inst-mr-apply` + AC; S5 endpoint map; PRD `fr-supersession` + §17.5 |
| H-3 (phase-axis unit hazard) | **CONFIRMED** | **D-89** — phase-blind counter stated; override preserves the D-82/D-98 field list (`PHASE_OVERRIDE_UNIT_MISMATCH`); fixture scenario | S2 `inst-ph-override-units` (new) + `inst-ph-usage-invariant` + §5 + DoD + AC; S3 `inst-tb-window-continuity` + fixture note; PRD `fr-plan-phases` |
| M-1 (current revision undefined) | **CONFIRMED** | **D-90** — revision flip-at-commit + partial `UNIQUE` one published revision; retire targets it | S1 §3.7 + §4.3; S2 §4 `inst-pl-supersede` (new) + AC; S11 `inst-rt-cancel` |
| M-2 (D-06 × D-86 unreconciled) | **CONFIRMED** | **D-91** — subject-typed read-model deltas; one row per overlay/membership publish unit | S1 §3.7 + §4.4; S9 §7; the D-86 entry amended |
| M-3 (bundle/overlay revisioning missing) | **CONFIRMED** | **D-92** — `plan_revision` on bundle child tables; overlay draft-revision rows | S8 §6 ×3 + AC; S9 §6 ×2 + constraints + AC; the D-83 entry extended |
| M-4 (boundary classification unrecomputable) | **CONFIRMED** | **D-93** — read-time classification by Subscriptions; stamp removed; **flagged for veto · joint** | S6 `inst-pc-boundary` + §6; PRD glossary + `fr-plan-change-contract` + AC #108; the D-25 entry revised |
| M-6 (gate key granularity) | **CONFIRMED** | **D-94** — conjunction over bound keys; exemption = plan-market sellability; **flagged for veto · joint** | S7 `inst-sg-conjunction` (new) + `inst-sg-joint` + `inst-fg-trailing` + DoD + AC; S5 `inst-mat-registered`; PRD `fr-sellability-gate` + §9.2 ×2 |
| M-7 (add-on coverage per currency) | **CONFIRMED** | **D-95** — per `(currency, region)` pair, one rule with S2 | S4 `inst-cb-addon` + DoD + AC; PRD `fr-invoice-currency-binding` |
| M-8 (`invoiceGroupingKey` homeless) | **CONFIRMED** | **D-96** — S2 plan column + read-model projection | S2 §6 + §1.8 |
| M-9 (override ref stale on supersession) | **CONFIRMED** | **D-97** — the ref binds the scope key; resolution through the supersession chain | S2 `inst-cmp-override-home`; PRD `fr-addon-rules` |
| L-6 (kind flip on supersession) | **CONFIRMED, escalated** | **D-98** — `model_kind` joins the preserved set; **flagged for veto** | S3 `inst-tb-supersession-units` + §5 + DoD + fixture + AC; S1 §4.3; PRD `fr-supersession` + §17.5; the D-82 entry amended |
| L-1…L-5, L-7…L-10 | **CONFIRMED** | Text fixes in place (see the [L] list above) | S1 §4.4/§3.7; S2 ×3; S5 ×2 + endpoint; S8; S9 §6; S10 |
