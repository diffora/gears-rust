<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — manual slice review (2026-07-31, third pass of the day)

**Scope**: all 12 slice designs ([`../design/`](../design/)) + [`DESIGN.md`](../DESIGN.md) +
[`DECISIONS.md`](../DECISIONS.md) (D-01…D-112, §F.1/§F.2) + targeted PRD verification + the
**cross-gear seam surfaces** (rating `design/03/05/09/13`, `DECISIONS.md` T-D-12, `SEAMS.md`;
subscriptions PRD §plan-change) — this wave deliberately read the consuming side of every
snapshot-frozen contract field, which no previous wave had done end-to-end.
**Method**: single-reviewer sequential pass, slice by slice, no subagents; every candidate
finding cross-checked against the decision register, the PRD text, and (for cross-gear claims)
the sibling gear's own documents before being claimed. Mechanical gate: `spec-check` over
pricing with `--auto-context` (rating + subscriptions loaded) — **0 findings**, 70 known-debt
suppressed (D-69 pin).
**Baseline**: everything already registered — the open product forks (§F.1), the carried
findings (§F.2), D-01…D-112 and their veto confirmations, and the owed cross-gear adoptions
already on the board (Rating D-78; Subscriptions SUB-P7/SUB-P8/D-93/D-94) — is deliberately
**not** re-reported.

**Verdict**: the intra-gear semantic layer is in the best shape it has ever been — seven waves
have closed the truth side, the read side, and the unit-guard family. What this pass finds
lives at the three boundaries the previous waves stopped short of: **(1) the cross-gear
boundary** — a snapshot-frozen field Rating and Subscriptions both consume and name pricing as
the home of does not exist anywhere in pricing (H-1, the D-01 class between gears); **(2) the
evaluation-domain boundary** — the materiality evaluator's delta is defined only over amount
columns, so every quantity-semantics and contract-field money lever auto-publishes below a
configured threshold (H-3, the D-50 class through the non-amount door); and **(3) the
resolution-boundary fine print** — pin-eligibility is not prefix-closed (H-2, the D-101
divergence one version out) and the plan-subject projection never says which rows/windows a
delta carries (M-6, where the only enumeration given actively drops the expired windows
arrears rating needs). The rest is the familiar sibling-surface tail: D-97's binding is
market-contradictory as written (M-1), D-89/D-84 miss the orphan phase-scoped line (M-2),
D-88 never reached bulk import (M-3), D-110 never reached bundles (M-4), the overlay scope
axes are only half-validated (M-5), and `package_size` is missing from the D-82/D-98/D-89
preserved set (M-7).
Totals: **3 [H]**, **7 [M]**, **6 [L]**.

> **Status (fix wave, same session, on the owner's go): ALL FINDINGS CONFIRMED AND FIXED.**
> Every finding was text-verified against the cited documents before being claimed — including
> the negative checks (rating SEAMS P1/P2 do **not** track H-1's field; no §F.1/§F.2 entry
> covers any item below) — and none was rejected or downgraded during the fix pass. H-1…H-3
> and M-1…M-7 are closed as **D-113…D-122** in [`DECISIONS.md`](../DECISIONS.md); the [L]
> items are text fixes in place. The per-finding mapping is in the
> [Verification & fix record](#verification--fix-record). `spec-check --auto-context` re-run
> after the wave: **clean**. **Flagged for veto: D-113 (joint with Rating + Subscriptions),
> D-115, D-117, D-118, D-119, D-122**; owed cross-gear adoptions: Rating — the D-113
> target-snapshot routing + per-line unit check + absence default (a SEAMS row), and the
> reconciliation of rating `design/09`'s "fail-closed when absent" to the decided
> default-reset.

Severity scale: **[H]** breaks money/correctness or is unimplementable as written · **[M]**
teams can build incompatible behavior · **[L]** contained.

---

## [H] Findings

### H-1. The plan-change tier-`Q`/pool **carry-vs-reset flag** does not exist in pricing — while Rating and Subscriptions both consume it "from the pinned snapshot"

The rating gear's plan-change semantics hang on a snapshot-frozen flag pricing never defines:

- rating `design/09-period-plan-change.md` §Frozen-inputs table names the owner explicitly:
  *"Pricing (Product Catalog) — `prorationBasis` (incl. `none`), `billingAnchorPolicy` + D-20
  clamp, floor/cap attachment, **carry-vs-reset flags — all in the pinned snapshot**"*, citing
  `../../pricing/docs/design/06-consumer-contracts.md`;
- rating T-D-12 (adopted 2026-07-11): window-activation/phase-conversion boundaries always
  carry, *"plan-change per carry-vs-reset flag"*; rating `design/03` §4.3 and `design/13` §266:
  *"only a plan-change boundary consults the **snapshot-frozen** carry-vs-reset flag (`reset`
  ⇒ `bandOffsetQ = 0`)"*; rating `design/05` does the same for commitment pools (*"consumed
  frozen from the snapshot (default reset unless marked carry)"*);
- rating PRD §1.4/§6.11/§17.2 (*"tier `Q` and commitment-pool carry-vs-reset across the
  boundary MUST follow **snapshot-frozen configuration**"*, *"default reset unless marked
  carry"*) and subscriptions PRD §plan-change (*"applies tier-`Q` / commitment carry-vs-reset
  per snapshot"*) both restate the dependency.

Pricing's S6 — the named home — publishes `billingAnchorPolicy`, `prorationBasis`,
`creditOnDowngrade`, `billingTiming`, the grant set and the change contract. **No
carry-vs-reset field exists in any pricing document**: no FR, no column, no validation rule,
no read-model projection, no §9.2 contract line (grep over the whole docs tree: the only
`carry` is D-45's allowance `rolloverPolicy`, a different mechanism). No SEAMS row tracks it
either — rating SEAMS P1/P2, which `design/09` cites, are the `prorationBasis` enum and the
anchor clamp, and the owed-adoptions list (D-78, SUB-P7/P8, D-93/D-94) does not contain it.
This is the D-01/D-65/D-79/D-87 class — the rule is written, the input does not exist —
**for the first time between gears**: every plan change with an open usage window today
evaluates against an absent flag, and rating's own two absence readings disagree (`design/09`
§Frozen-inputs: *"all fail-closed when absent"* — every mid-window plan change fails closed;
rating PRD §17.2: *"default reset unless marked carry"* — silently resets). Two
implementations of the same seam diverge on money.

**Fix direction** (a joint decision with Rating + Subscriptions, three parts):
(a) give the flag a pricing home — most plausibly per plan (or per `allowedChangeTargets`
edge), authored in S6, snapshot-frozen, projected; state **which side's snapshot** carries it
at a change boundary (source vs target — rating never says);
(b) **unit guard for `carry`**: a carry across plans is the fourth door of the ×24 class
(D-82 closed supersession, D-98 the kind flip, D-89 the phase axis) — the counter key
`(subscription, meter, dimensionKey, window)` is plan-blind, so a `carry` between a `per_hour`
line on plan A and a `per_day` line on plan B applies an hours-denominated `Q` to
day-denominated bands, and no publish-time check can exist (independent plans). Either
`carry` is honoured only where the shared `(meter, dimensionKey)` line's D-82/D-98 field set
matches at change time (checked by the executor against both frozen snapshots, reset + operator
signal otherwise), or `carry` is constrained at authoring to explicit target edges validated
per line;
(c) reconcile rating's absence semantics with the chosen default (one of "fail-closed" /
"default reset" survives).

### H-2. Pin-eligibility is not prefix-closed — a stuck older version's late warm re-opens the D-101 replay divergence one version out

D-101 made pin-eligibility version-level: a version is pin-eligible once
`CatalogVersionPublished` has fired **"and every subject row that version projects carries its
warm-completion marker"** ([`01-foundation.md`](../design/01-foundation.md) §4.4; PRD
`fr-consumer-readmodel-resolution`). But resolution is still *"the subject's greatest
completed version ≤ the pin"* — it reaches **through** the pinned version into older deltas —
and nothing requires the older versions to be complete before a newer one becomes
pin-eligible. Warms are per-subject and parallel (D-91's whole point), and after a degraded
publish a re-drive *"continues past the SLO"* with no bound (§1.2/§3.6).

Scenario: `V5` (touching plan `A`) goes degraded, its re-drive outstanding; `V6` (touching
only plan `B`) publishes and warms fully → `V6` is pin-eligible by the letter of D-101. A
consumer pins `V6` and resolves `A` at its greatest completed delta ≤ `V6` — which is `A@V4`.
When `V5`'s re-drive completes, the **same pin** resolves `A@V5`. §4.4's supporting claim —
*"untouched subjects resolve their own older deltas, **which are frozen and never change**"* —
is false in exactly this case: the older delta existed-but-unwarm and completes later. One pin
resolves two contents over time; a rating run replayed at `V6` yields different money — the
precise defect D-101 was raised to close, one version out. (The `changeover_unwarmed` Critical
still fires for `V5`'s successor rows, but an alarm does not restore replay determinism, and
`pin_eligibility_overdue` says nothing about `V6`, which is "eligible".)

**Fix**: one sentence in §4.4 + the PRD — a version is pin-eligible only once **every version
≤ it** is pin-eligible (pin-eligibility is a monotonic frontier). This preserves D-91's
parallel warm storage, keeps "consumers keep pinning the previous pin-eligible version"
well-defined (the frontier edge), and makes the frozen-delta claim true by construction. The
cost is honest: a stuck version holds the frontier — which is what the existing
`pin_eligibility_overdue` Critical is for.

### H-3. The materiality evaluator's delta domain is amount-only — every non-amount money lever auto-publishes below a configured threshold

PRD `fr-approval-threshold-policy`: materiality is *"an absolute amount or percentage delta
per currency"*, per affected row, any-row-trips; S5 `inst-mat-percurrency` computes *"per-row
deltas in each row's own currency"*. Three legs, in decreasing order of bluntness:

- **(a) The delta is literally undefined for band-kind rows.** On `graduated`/`volume`/
  `package` rows `amount_minor` is NULL by construction (`AMOUNT_PLACEMENT_INVALID` — the
  money lives in `pricing_price_tier_band.unit_price_minor` / `package_price_minor`). Neither
  the FR nor S5 says what "the row's delta" is when the row's price is a band vector — band-wise
  against which pairing when the geometry changed? A percent policy against a `$0`/NULL
  baseline divides by zero. Teams will implement different (or vacuous) evaluations for
  exactly the rows that carry tiered revenue.
- **(b) Quantity-semantics fields move money at zero amount delta.** A supersession may
  legally change, with every amount column identical: `manual_quantity` (a `per_unit manual`
  row at 10 → 1000 seats = **×100 the charge**), `package_size` (blocks denominator),
  tier-band **bounds** (`[0,1000)` → `[0,10)` moves nearly all quantity into the paid band),
  `includedAllowance.quantity` (compiled `$0`-band width). None of these is in the D-82/D-98
  preserved set (they are the legitimate *content* of a price change), none produces an
  amount delta, so with a threshold configured each evaluates `auto_publishable` — one
  person, no approval record, no content pin.
- **(c) Contract and shape fields have no numeric delta at all.** Row fields: `billingTiming`
  (advance ↔ arrears — Billing's sole deferral input), `prorationBasis`,
  `billing_anchor_policy`, `credit_on_downgrade`, `tax_inclusive`/`tax_category_ref`,
  `quantity_source`. Plan-shape (a revision with unchanged rows): descriptor set (**GL
  code**, line template, itemization rule), phase graph/durations (trial 7 → 90 days),
  add-on rule set (required flips, `depends_on`, `price_override_ref`), cycle/frequency,
  availability dates, `PlanTier` override, `invoiceGroupingKey`. A pure-shape change set
  contains zero price rows, so nothing trips.

The register has closed this exact class four times — D-50 (overlays), D-104 (bundle
composition/rev-share, whose rationale *"changes what the customer receives at an unchanged
price"* applies verbatim to (b) and (c)), D-62 (windows), D-109 (retirement) — and S10's grant
non-price fields got the G1 treatment explicitly (*"no numeric delta ⇒ always material"*,
`inst-pg-material`). The plan-shape and row-field siblings never did.

**Fix direction**: define the delta domain normatively and close the residue with the G1
principle generalized: (i) for band-kind rows the delta = the per-band amount vector compared
band-wise **iff geometry is unchanged**; (ii) any change to quantity-determining/geometry
fields (band bounds, `package_size`, `manual_quantity`, allowance quantity) and (iii) any
contract/shape field change with no numeric delta ⇒ **material** (registered trigger or a
blanket "a change whose effective-price delta is not computable catalog-side is material").
Percent-over-zero-baseline ⇒ material. One register entry + S5 `inst-mat-percurrency`/
`inst-mat-registered` + PRD FR.

---

## [M] Findings

### M-1. `price_override_ref` "binds that row's canonical scope key" — but one key cannot cover every market the rule requires it to

S2 `inst-cmp-override-home` + PRD `fr-addon-rules` (D-97): the ref *"binds that row's
canonical scope key, not the id"* and in the same sentence *"publish of the base plan
validates the referenced **key** is published and **covers every `(currency, region)` the
base plan sells**"*. A canonical scope key **fixes** one `(currency, region)` — the two
clauses are unsatisfiable together for any multi-market base plan, and resolution for a
subscriber on a market other than the bound key's has no defined path (D-97's "follows the
key through windows" has no key to follow there). The intended object is evidently the
**key-family modulo the market axes** — `(addon planId, priceOverlay, phase,
priceEligibility, chargeKind, cohort)` with `(currency, region)` free, the subscriber's bound
market selecting the member. **Fix**: state the binding as key-modulo-market (and the D-95
coverage walk as ranging over the family's members); one sentence in S2 + the FR + the D-97
entry.

### M-2. A phase-scoped usage row with no terminal-phase row of the same `(meter, dimensionKey)` line — the orphan override

`inst-ph-usage-invariant` defines the phase-scoped row as an **override** (*"overrides it for
its phase"*) but nothing requires the terminal-phase base row to exist. For an orphan line
(priced only on a non-terminal phase): D-89's guard loses its referent (`inst-ph-override-units`
compares against *"the phase-invariant terminal-phase row of its line"* — absent; pass
vacuously? fail?), D-84's completeness never sees it (*"evaluated over its phase-invariant
terminal-phase rows; phase-scoped overrides are additive and exempt"*), and after the phase
converts, the line resolves to **nothing** — usage that continues into the next phase fails
closed on a published, sellable plan. The "sold but unrateable" state D-15/D-64/D-84 close on
three other doors, through the override door. **Fix**: either forbid the orphan
(`PHASE_OVERRIDE_ORPHANED`, 422 — an override requires its terminal-phase base line) or define
the phase-limited line as first-class (its own D-84 per-market completeness; a D-89 rule with
no comparison target = the line must be explicitly terminal-phase-absent, and conversion-time
resolution to nothing is then an authored fact). The forbid is the D-53 posture and is one
rule.

### M-3. Bulk import's commit target is undefined against append-only published rows — D-88 never reached the import path

`fr-bulk-price-import`/S12 commit *"per-row under optimistic locks (ETag)"* — but a published
row cannot be UPDATEd (append-only REVOKE + trigger), and the only sanctioned change paths are
the D-88 supersession unit and the cutover, both with window operations and a
changeover-instant floor. Mass repricing — the import's sibling — was explicitly rebuilt on
supersession units with the run-level instant (`inst-mr-api`/`inst-mr-apply`, D-88/D-111);
the import got neither an instant in its API nor any unit language, yet its concurrency story
(ETag conflicts with "concurrent manual edits", the bulk lock on **published** rows in
`pricing_bulk_row_lock`) only makes sense if it can touch published rows. As written, one team
builds import-as-draft-authoring (ETags over draft rows only; publish separately) and another
builds import-as-bulk-supersession missing the instant floor and the window ops — incompatible,
and the second reopens the transient fail-closed window D-88 closed. **Fix**: state the
import's domain — either **draft-only** (rows land as drafts / new-key rows; changing a
published row via import is rejected, routed to a repricing run), or give the import the full
D-88 treatment (per-row supersession units + one import-level changeover instant bounded at
approval commit). Draft-only matches "import" semantics and keeps one bulk-change mechanism.

### M-4. Mixed tax display basis across a bundle's components — the D-110 hole one composition level up

D-110 pinned **one display basis per `(currency, region)` per plan** (`TAX_BASIS_MIXED_MARKET`),
rationale: *"an invoice is one document"*. A bundle composes several plans onto exactly one
invoice: component A tax-inclusive in EU, component B tax-exclusive in EU passes every S8
check (coverage checks currency/region/frequency only — `inst-bc-coverage`), and pre-Tax-Engine
the D-94 conjunction masks it only *incidentally* (A's flagged market blocks the bundle). The
moment Tax Engine GAs and A re-publishes clear, the bundle sells a mixed-basis invoice — the
exact artifact D-110 forbids within one plan, assembled across plans. **Fix**: bundle publish
validates `tax_inclusive` uniformity across all component rows (+ own rows for `own_price`)
per sold market — the D-110 rule with the component walk S8 already performs — or an explicit
deferral naming the pre-GA mask. Sibling-surface class; one rule in S8 + a D-110 entry note.

### M-5. Overlay scope values are validated for `brand`/`customerGroup` only — `region` is unvalidated and un-guarded, `partner`/`orgTier` have no value universe at all

`inst-plv-scope` names two validations: brand → S4 taxonomy, customerGroup → `GroupTaxonomy`.
For a **region-scoped** overlay nothing validates `scope_value` against
`pricing_region_taxonomy`, and S4's retire guard (`inst-tx-mutation`) enumerates *"an active
published price row (`region`) or an active brand-scoped `PriceOverlay` scope (`brand`)"* — a
region value retires cleanly while region-scoped overlays still name it (dangling scope,
silently matching nothing downstream). For **partner** and **orgTier** no document in the set
(PRD glossary included — it defines the *concepts*) declares where legal values come from, so
the scope value is a free-form string — the §F.2 `rounding_policy_ref` pattern on the axis
that selects **who gets an adjustment** — and no contract names the payer→partner/orgTier
resolution input Tariffs matches against. **Fix**: (i) region scope values validate against
the region taxonomy and join its retire guard (one clause each in S9 + S4); (ii) partner/orgTier
either get a declared universe (registry/AMS-backed reference or a tenant taxonomy like
brand) + a `(payer → partner/orgTier)` resolution lane named in the Tariffs contract, or an
explicit free-form statement with the Tariffs-side matching input pinned.

### M-6. The plan-subject projection never says which rows and windows a delta carries — and its only stated enumeration drops the expired windows arrears rating needs

Foundation §4.4 (D-99): a plan subject carries *"per canonical scope key, the key's
`PriceWindow` intervals and states (`[effectiveFrom, effectiveTo)` + **`scheduled | active`**)"*.
Nothing anywhere defines the projected **row set** (which `lifecycle_state`s; whether
superseded-but-window-live predecessors are carried — they must be, for pre-changeover
resolution; how far back window history reaches), and the state enumeration as written
**excludes `expired`**. Consequence: rating pins a current version and rates *past* instants
(arrears always lag). Immediately after any changeover, the predecessor's window is `expired`;
if any publish unit re-projects the plan (a descriptor fix, a window mutation — anything), the
new delta, projected per the stated enumeration, **omits** the expired window — and the
greatest-completed-≤-pin rule now serves that delta, so resolution at yesterday's `t` finds no
covering window and fails closed on a legitimately covered period. (Before the re-projection
the *older* delta still carried the interval — which is exactly why this hides in testing.)
The inverse reading — project everything forever — is the perf hazard instead: every delta
re-carries the plan's full accumulated row+window history, so delta size grows linearly with
plan age and total storage quadratically with publish count, unbounded at the ≥ 7y retention.
**Fix**: one normative sentence defining the projected set — e.g. every row (any lifecycle
state) whose window intersects `[projection_time − H, ∞)` with `H` ≥ the longest billing cycle
sold + the close/correction lag (rating supplies its replay horizon; older re-rates replay from
older pins per `fr-pricing-snapshot`) — and the delta-size consequence stated in §10.

### M-7. `package_size` is missing from the D-82/D-98/D-89 preserved field set

The preserved set on usage-row supersession/phase-override is `meter`, `dimensionKey`,
`model_kind`, `billingGranularity`, `aggregationFunction`, `aggregationGranularity`,
`tierAggregationWindow`, `tierQualificationWindow`. `package_size` is not in it — while
D-58's own argument makes block math **non-linear in the window** (`blocks =
ceil(used / packageSize)` over the accumulated `used`), and D-98 banned mid-window kind flips
precisely because non-incremental math *"re-prices the already-accumulated window total under
new math"*. A `package → package` supersession (or phase override) changing `package_size`
100 → 10 mid-window re-buckets every unit already consumed — the same retroactivity, inside
one kind. (`package_price_minor` is the legitimate price lever — the analogue of a band's
`unit_price_minor`; the block **size** is quantity semantics, the analogue of band bounds —
which are equally unguarded, folded into H-3(b) since they are also the *intended* content of
a price change; `package_size` differs in having a clean same-kind continuity argument, hence
its own finding.) **Fix**: add `package_size` to the preserved set in S3
`inst-tb-supersession-units` + S2 `inst-ph-override-units` + both codes' descriptions + the
negative fixture scenario; amend the D-82 entry.

---

## [L] Findings

- **L-1.** S2 `inst-cmp-usagetype`: the priced-dimension subset check (*"the dimension set …
  MUST be a subset of the UsageType's declared `metadata_fields` keys"*) never exempts the
  `dimension_key = ''` empty-tuple sentinel (S3 §6) — `''` is never a declared key, so an
  undimensioned row on a bound meter fails `METER_DIMENSION_UNDECLARED` read literally. Say
  the sentinel is exempt (an undimensioned row prices the whole meter).
- **L-2.** Physical append-only enforcement (REVOKE + column-whitelist trigger) is declared
  for `pricing_price`, `pricing_price_window`, `pricing_historical_price` and the audit log —
  published **plan revisions and every revision-scoped child/composition table** (phases,
  add-on rules, descriptors, grants, composite meters, bundle tables, overlay revisions) are
  immutable by convention only (S1 §3.7 says so explicitly for `pricing_plan`). The projector
  re-drive reads truth rows, so an unsanctioned UPDATE silently changes a frozen version at
  re-warm. State the same trigger discipline for published revision rows + their children, or
  record the waiver and why.
- **L-3.** Component `one_time_setup` (and `one_time`) rows under a bundle purchase are
  unstated: `inst-bb-sum` covers recurring amounts and usage rating; whether a component's
  setup row charges at bundle activation (and per `inst-cs-setup-timing` whose activation) is
  silent. One sentence in S8.
- **L-4.** A **phased** component plan in a bundle is unstated: which phase's rows sum
  (terminal only?), and whether a bundle subscription runs the component's phase schedule at
  all. Either forbid phased components at launch (`COMPONENT_PHASED`, the D-53 posture) or
  define the semantics jointly with Subscriptions.
- **L-5.** Cutover/supersession composition on a **dormant key** (no active/open window at
  compose — coverage ended) has no stated behavior; `inst-co-shorten`/`inst-su-compose`
  presuppose a current window to shorten. Presumably `CUTOVER_GAP`/compose rejection +
  revival via plain publish + schedule; say it.
- **L-6.** The compiled `carry` allowance grant's issuance scope per subscription market is
  implicit: `pricing_plan_grant.source_price_id` is recorded as *"replay lineage"* only, while
  a multi-market plan compiles one grant per allowance-carrying row — nothing states Billing
  issues **only** the grant whose source row is the subscription's bound market row. One
  clause in `inst-ac-carry`.

---

## Performance assessment

The publish path, the activation job, the repricing journal and the D-111 validation split
remain sound; the read path is bounded post-D-112. This wave's material points:

- **M-6 is the read-model's open cost question**: until the projected row/window set is
  defined, the delta-size model is undefined between "loses arrears coverage" and "grows with
  the plan's whole history". The fix's horizon parameter is also the size bound — worth
  stating in the same sentence.
- **`overlay_index` write amplification and serialization** (D-112): every overlay publish
  unit rewrites the tenant-singleton index row, whose size is O(live overlays); concurrent
  overlay commits serialize on it. Fine at expected cardinalities (an explicit cap on live
  overlays per tenant would make it a non-question); the read side is exactly as D-112 sized
  it.
- **H-3's fix has a perf edge worth keeping**: band-wise delta comparison is O(bands) per row
  at submit — trivial — but the "geometry changed ⇒ material" rule avoids ever needing an
  effective-price evaluation catalog-side (which the no-charge-computation principle forbids
  anyway).
- **H-1's fix adds no hot-path cost**: the flag freezes into the snapshot like its S6
  siblings; the `carry` unit-compatibility check runs at change time on two already-frozen
  snapshots (executor-side, O(shared lines)).
- Unchanged and still sound: order-time = one plan-subject read + one `overlay_index` read +
  per-matching-overlay document reads + membership probe; repricing O(rows) under D-111 with
  the aggregate pass amortized per plan; the D-79 lane synchronous, fail-closed, bounded per
  mutating commit.

## Per-slice verdicts

| Slice | Verdict |
|-------|---------|
| 01-foundation | H-2 and M-6 are rooted here (§4.4 pin-eligibility + projection content); L-2 |
| 02-plan-definition | M-1 (co-owner), M-2, H-3(c) shape fields; L-1; the phase/revision machinery is otherwise tight |
| 03-price-structure | M-7, H-3(a/b) band-kind deltas; the D-77/D-82/D-89/D-98 family itself is accurately landed |
| 04-currency-tax | M-5 (retire-guard half); D-95/D-110 landed cleanly |
| 05-governance | H-3 is rooted here (`inst-mat-percurrency` delta domain); the approval/authz/audit surface remains the most mature in the set |
| 06-consumer-contracts | **H-1 lands here** (the missing carry-vs-reset home); otherwise clean — D-93's read-time move is accurate |
| 07-pricewindow-linkage | H-2 (consumer side), L-5; the D-88/D-99/D-100/D-101 machinery reads coherently |
| 08-bundles | M-4, L-3, L-4; D-104/D-105/D-92 landed accurately |
| 09-price-overlays | M-5; D-42/D-78/D-107/D-112 landed accurately |
| 10-advanced-primitives | L-6; D-106 landed; the allowance-compile chain is the most corner-case-complete algorithm in the set |
| 11-lifecycle | Clean this pass — the synthesis chain (D-76/D-81/D-87/D-102) and D-108/D-109 close their surfaces; H-1 touches its migration contract (counter across `PlanLink`) |
| 12-operator-efficiency | M-3; D-111/D-35/D-37 are consistent |

## Recommended register actions

- **H-1** → a DECISIONS.md entry, **joint with Rating + Subscriptions** (field home + which
  snapshot + absence default + the `carry` unit guard); a new SEAMS row rating-side; S6 field +
  FR + §9.2 contract line.
- **H-2** → a DECISIONS.md entry amending D-101 (prefix-closed frontier); S1 §4.4 + PRD
  `fr-consumer-readmodel-resolution` one sentence each.
- **H-3** → a DECISIONS.md entry (delta domain + G1 generalization); S5
  `inst-mat-percurrency`/`inst-mat-registered`; PRD `fr-approval-threshold-policy`.
- **M-1 … M-5, M-7** → per-finding register entries + the named slice rules (each is one to
  three sentences at its cited location); M-3 and M-4 carry a product flavor (import domain;
  bundle basis rule) — flag for veto.
- **M-6** → a DECISIONS.md entry (projected-set definition + horizon + size model); S1 §4.4 +
  §3.7 + S7 `inst-sg-surface`.
- **[L] items** → text fixes in place at the cited instructions.

---

## Verification & fix record

Fix pass, same session, on the owner's go: every finding re-checked against the cited document
text as its fix was applied. **All 3 [H] + 7 [M] + 6 [L] CONFIRMED**; none rejected, none
downgraded.

Verification notes worth keeping. For **H-1** the decisive negative check was rating
`SEAMS.md`: P1/P2 — the rows rating `design/09` cites beside the flag — are the `prorationBasis`
enum and the anchor clamp, so no seam tracked the field and it was a live gap, not an owed
adoption; rating's own absence-semantics split (`design/09` "fail-closed when absent" vs PRD
"default reset") upgraded it from "missing field" to "two implementations diverge on money".
For **H-2** the falsifying sentence was §4.4's own *"untouched subjects resolve their own older
deltas, which are frozen and never change"* — an existing-but-unwarm older delta completes
later, so the claim fails exactly where the fallback needs it. For **H-3** the arbitration was
`AMOUNT_PLACEMENT_INVALID`: the design itself forces `amount_minor` NULL on band kinds, so the
FR's "the row's delta" provably had no operand for them. For **M-6** both readings were checked
before deciding: the `scheduled | active` enumeration loses the arrears tail on the first
re-projection after a changeover, and the everything-forever reading grows deltas with plan
age — the horizon is the only shape that serves both, and old-`t` resolution correctly falls
back to replay-from-old-pins because deltas are retained on the truth horizon (D-86). For
**M-7**, rating T-D-12's *"package counts blocks by cumulative ceil-diff"* is what makes a
mid-window `package_size` change formally ill-defined, not merely retroactive — the strongest
form of the argument came from the consuming gear's own math.

| Finding | Verdict | Fix | Where it landed |
|---------|---------|-----|-----------------|
| H-1 (carry-vs-reset flag nowhere in pricing) | **CONFIRMED** | **D-113** — `usageCounterOnPlanChange {reset (default) \| carry}` on the plan, target-snapshot routing, absence = reset, `carry` per unit-matched shared line only; pool flag explicitly not published; **flagged for veto · joint** | S6 `inst-pc-counter-carry` (new) + §6 + DoD + AC; PRD `fr-plan-change-contract` + `contract-tariffs-readmodel` |
| H-2 (pin-eligibility not prefix-closed) | **CONFIRMED** | **D-114** — pin-eligibility is a monotonic frontier (every earlier version pin-eligible); amends D-101 | S1 §1.2 + §3.6 + §4.4; S7 AC (stuck-older-version scenario); PRD ×2 sites; the D-101 entry pointer |
| H-3 (materiality delta domain amount-only) | **CONFIRMED** | **D-115** — delta domain per kind (band-wise iff geometry unchanged); geometry/quantity changes material; no-computable-delta trigger over row contract fields + plan shape; **flagged for veto** | S5 `inst-mat-percurrency` + `inst-mat-registered` + DoD + AC; PRD `fr-approval-threshold-policy` |
| M-1 (override ref binds one key vs all markets) | **CONFIRMED** | **D-116** — the ref binds the key **family modulo market**; the bound market selects the member; amends D-97 | S2 `inst-cmp-override-home`; PRD `fr-addon-rules`; the D-97 entry pointer |
| M-2 (orphan phase-scoped usage override) | **CONFIRMED** | **D-117** — `PHASE_OVERRIDE_ORPHANED`; phase-limited lines a named Future gate; **flagged for veto** | S2 `inst-ph-usage-invariant` + §5 + DoD + AC; PRD `fr-plan-phases` |
| M-3 (bulk import vs published rows undefined) | **CONFIRMED** | **D-118** — the import is draft-plane authoring; `IMPORT_TARGETS_PUBLISHED`, remediation = a repricing run; **flagged for veto** | S12 flow + `inst-bk-phase1` + §5 code + DoD + AC; PRD `fr-bulk-price-import` |
| M-4 (bundle mixed tax basis) | **CONFIRMED** | **D-119** — `BUNDLE_TAX_BASIS_MIXED` per bundle-market + the D-54-pattern reverse guard on component re-publishes; **flagged for veto** | S8 `inst-bc-taxbasis` (new) + §5 + DoD + AC; S4 `inst-td-basis-uniform` cross-ref; PRD `fr-bundle-composition` |
| M-5 (overlay scope-value universes) | **CONFIRMED** | **D-120** — region validates + joins the retire guard; partner/orgTier get tenant taxonomies (the D-01 pattern) + a needs-decision on the payer-resolution lane | S9 `inst-plv-scope`; S4 ×5 sites (`inst-tx-brand`/`inst-tx-mutation`/§5/§6/DoD); PRD `fr-priceoverlay-authoring` + §17.4 row |
| M-6 (projection row/window set undefined) | **CONFIRMED** | **D-121** — projected set = rows whose windows intersect `[projection − H, ∞)`, `H` = 2 × longest cycle sold; states incl. `expired`; older `t` replays from old pins; size model stated | S1 §4.4 + §3.7; PRD `contract-tariffs-readmodel` |
| M-7 (`package_size` not preserved) | **CONFIRMED** | **D-122** — joins the D-82/D-98/D-89 preserved set (supersession + phase override); negative fixture scenario; **flagged for veto** | S3 ×5 sites; S2 ×2 sites; S1 §4.3; PRD `fr-supersession` + `fr-plan-phases`; the D-82 entry pointer |
| L-1 (dimension sentinel vs subset check) | **CONFIRMED** | `''` exempt — an undimensioned row prices the whole meter | S2 `inst-cmp-usagetype` |
| L-2 (physical guard scope) | **CONFIRMED** | The REVOKE + column-whitelist discipline extended to published revision rows + every revision-scoped child/composition table | S1 §3.7 |
| L-3 (component setup rows under a bundle) | **CONFIRMED** | Never charge — setup is tied to standalone activation; the bundle's own setup row is the only activation charge | S8 `inst-bb-sum`; PRD `fr-bundle-composition` |
| L-4 (phased components) | **CONFIRMED** | Forbidden at launch (`COMPONENT_PHASED`) — a named Future gate, the D-53 posture | S8 `inst-bb-sum` + §5 code + DoD + AC; PRD `fr-bundle-composition` |
| L-5 (dormant-key cutover/supersession) | **CONFIRMED** | Compose fails (`CUTOVER_GAP` / gap-freeness) — revival is plain publish + schedule | S7 `inst-co-shorten` + `inst-su-compose` |
| L-6 (carry-grant issuance scope) | **CONFIRMED** | `source_price_id` is normative issuance scope — a subscription receives only its bound market row's grant | S10 `inst-ac-carry` |
