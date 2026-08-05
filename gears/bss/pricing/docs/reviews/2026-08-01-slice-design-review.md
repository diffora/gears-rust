<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — manual slice review (2026-08-01, fifth sequential pass)

**Scope**: all 12 slice designs ([`../design/`](../design/)) read end to end, plus
[`DESIGN.md`](../DESIGN.md), [`DECISIONS.md`](../DECISIONS.md) (D-01…D-125), the three ADRs,
and the parts of [`PRD.md`](../PRD.md) each finding depends on (§9.2 contracts, §17.5 change
mechanisms + `CatalogVersion` increment table, §6.6). Cross-gear claims were checked against
the consuming gears' own text (`rating/docs`, `subscriptions/docs`).

**Method**: single reviewer, one sequential pass, no sub-agents. Every finding below was
verified against the document text before being claimed; the "Checked and refuted" section
records what did **not** survive, including two of the six items the 2026-07-31d pass carried
forward unverified.

**Mechanical baseline**: `spec-check --auto-context` over `gears/bss/pricing/docs` — **0
findings** (68 known-debt suppressions, tracked as D-69). Nothing below is a propagation or
id-hygiene defect; this pass is about semantics, corner cases, and cost.

**Lens** (deliberately different from the four earlier passes, which covered propagation,
mechanism adjacency, the read side, and the consuming side of snapshot-frozen fields):
**the failure and re-entry paths** — what happens the *second* time a mechanism runs, what
happens when a step fails midway, and what each rule costs at scale.

Severity: **[H]** breaks money/correctness or is unimplementable as written · **[M]** teams
can build incompatible behavior · **Cleanup/Minor** contained, latent, or hygiene.

Totals: **4 [H]**, **9 [M]**, **8 Cleanup**, 4 checked-and-refuted.

> **Status (fix wave 2026-08-01, on the owner's go): ALL FINDINGS FIXED.**
> H-1…H-4 → **D-126…D-129**; M-1…M-9 → **D-130…D-138**; C-1…C-8 applied in place with no D
> number. The per-finding mapping is in the [Verification & fix record](#verification--fix-record).
> **Veto round 2026-08-01 (per-item, product owner, same day): D-126, D-132, D-138 — ALL
> CONFIRMED as decided**, each against its stated alternatives — D-126 keeps the fall-through
> to `all_subscriptions` for a pin whose closing instant carries no generation (the strict
> fail-closed alternative would have made every routine supersession on an occupied key an
> incident source until re-bind); D-132 accepts a grandfathered cohort keeping a display basis
> or anchor the current rows have left, rather than letting one cutover disable tax/anchor
> migration for a market indefinitely, and **without** widening the published-row mutation
> whitelist; D-138 keeps `fixed` as a **replacement** rather than cutting it to Future or
> making it an additive synonym of `markup`. **Nothing in pricing awaits veto.**
> **Owed cross-gear adoptions** (now unqualified): Rating — the D-126 cohort bootstrap
> (`CohortGenerationSelector`/`CohortPin` + a SEAMS row) and D-138's `fixed` semantics (step-4
> line application + the `maxCumulativeMarkup` interaction); Subscriptions — D-131's lane
> response shape (SUB-P8, already owed; this changes its shape, not its trigger).

---

## [H] Findings

### H-1. The first grandfathered generation is unbindable — the cohort selector resolves zero rows for exactly the population grandfathering protects

ADR-0002 and `inst-el-generation` fix generation selection as: *within `existing_grandfathered`,
resolve the row whose `cohort` equals the cohort of the subscription's **pinned price id**
(`pricingSnapshotRef` already pins it — no separate binding store)*. Rating adopted it verbatim
(`CohortGenerationSelector`, rating `design/02` §4.2/§190-191; rating PRD §475, §549, §1312).

A subscription that predates the **first** cutover on a key has a pinned price id on the
*predecessor* row, whose `cohort` is `none` — publish validation enforces
`cohort ≠ none ⇔ priceEligibility = existing_grandfathered` (ADR-0002, Foundation §4.1), so a
grandfathered row can never carry `none`. The only generation after the cutover is
`cohort = T1`. `none ≠ T1`, so the selector keeps **zero** rows, and rating's own rule then
applies: *"If no eligible price applies, evaluation MUST fail (no silent fallback)."*

The class filter makes it worse rather than better: rating's `EligibilityClassFilter`
*promotes* the subscriber into `existing_grandfathered` ("includes **only** subscriptions
activated before cutover"), and the class order `existing_grandfathered >
new_subscriptions_only > all_subscriptions` has already excluded the successor — so the
subscriber lands in a class whose generation selector rejects every candidate.

The rule is self-consistent only **from the second cutover onward**, when the pin already
carries a generation cohort. Nothing in either gear defines the bootstrap: the snapshot is
immutable so the cutover cannot re-pin (Foundation §4.4), there is no "`cohort = none` ⇒ the
earliest generation whose instant post-dates the pinned row's window" rule, and rating's
`CohortPin` contemplates only *absence* ("a torn-pin failure"), never the value `none`.

Failure: at the **first** cutover on any key, every pre-cutover subscriber's next
rating/renewal resolution fails closed — or, on a looser implementation that falls through the
empty class, silently resolves the `all_subscriptions` successor at the **new, higher** price,
which is precisely the outcome the whole ADR-0002 machinery exists to prevent. Note the
asymmetry with D-04, which closed the *exit* hole (re-bind lags `grandfatherUntil` expiry by up
to one cycle); the *entry* hole was never examined.

**Fix shapes** (product/architecture call, joint with Rating): (a) define the bootstrap
selection — `cohort = none` on the pin ⇒ the generation whose cutover instant is the earliest
one at or after the pinned row's window end (deterministic, snapshot-only, needs no new
store); or (b) first hop by `activatedAt` vs cutover, cohort-pin thereafter (rating already
carries `activatedAt` for the class filter). (a) is preferred — it keeps one input.

### H-2. The grandfathering cutover's successor escapes the D-82/D-98/D-122 supersession unit guard

D-100 established the cutover as the **second** sanctioned producer of
`published → superseded` on the *same* canonical scope key (`inst-co-supersede`,
`inst-ps-supersede`). S3 `inst-tb-window-continuity` keys the tier counter `Q` on
`(subscription, meter, dimensionKey, window)` — plan-blind, phase-blind, and *row*-blind — so
the counter continues across a cutover changeover exactly as it does across a supersession.

The unit guard is written as *"a **usage-row supersession** MUST NOT change the fields the
continued `Q` is denominated in"* (S3 `inst-tb-supersession-units`) and is invoked from exactly
one place: `inst-su-compose` step (a). `algo-cutover` — `inst-co-shorten`, `inst-co-copy`,
`inst-co-successor`, `inst-co-atomic`, `inst-co-supersede`, `inst-co-bounds` — never cites it.
The cutover's successor arrives as a client-authored "successor row ref" (`inst-gc-api`), and
no rule says it sets `supersedes_price_id`, so even a generically-registered pipeline rule has
no predecessor to compare against.

Failure: a cutover whose successor flips `per_hour → per_day` (or `graduated → volume`, or a
`package_size`) applies an hours-denominated continued `Q` to day-denominated bands — the ×24
band-edge class the register has now closed four times (supersession D-82, kind flip D-98,
phase axis D-89, plan change D-113), reopened through the fifth door, and on the one path that
is *always material* and therefore feels safest.

**Fix shape**: state the guard on the cutover successor in `algo-cutover` step 3, or — better —
hoist it to "any row landing on an occupied published key", make both units set
`supersedes_price_id`, and add the cutover negative scenario to the `supersession_continuity`
fixture (S3 §6) beside the existing supersession and phase-conversion scenarios.

### H-3. Plan retirement is consumer-visible, is not a publish unit, and after it the projector has no source

Two halves, both load-bearing.

**(a) No publish unit.** Sellability predicate (4) reads the plan **lifecycle state**, and
`inst-sg-pinned` requires all six predicates to be point-in-time evaluable *from the pinned
read model*. `inst-rt-event` discharges this as *"the read model flags the plan
not-sellable"* — an in-place mutation of a frozen version, which D-85/D-99 doctrine forbids
outright ("a frozen `CatalogVersion` never mutates"). PRD §17.5's increment table has four
change classes — price-only (incl. windows, D-99), structural, overlay/membership, draft-only —
and retirement is in none of them; no slice declares it as requesting a pending
`CatalogVersion` or re-projecting the plan subject. This is D-99's shape one surface over.

It cannot self-heal. A retired plan can never publish again — the state machine has no
`retired → draft`/`published` edge (`inst-pl-norollback`) and the open draft revision is
**deleted** in the retirement transaction (`inst-rt-cancel`) — so no later publish can ever
re-project the plan subject. The read model advertises the retired plan as sellable
**permanently**. The D-99 window-cancel path does not rescue it either: under D-51 retirement
cancels windows *only* on keys with no in-flight subscribers, so a plan with subscribers
everywhere cancels nothing and triggers no publish unit at all.

**(b) And if it did re-project, the source rule breaks the other way.** Foundation §4.4 sources
the projection from *"the **published** revision's own truth rows"*, and D-90's own
justification names "the published revision" as the input of "the projector (D-83), the
**sellability lifecycle predicate**, and every referential check". After the
`published → retired` flip there is no published revision. A re-warm or a degraded re-drive
would find no source and project the plan subject **empty** — breaking rating resolution for
in-flight subscribers, i.e. the exact guarantee D-51 exists to preserve ("retirement stops
selling, never rating").

**Fix shape**: make retirement a publish unit (validation → pending `CatalogVersion` ref →
plan-subject re-projection, the D-99 treatment), project `lifecycle_state` as a plan-subject
field, and restate the projector's source as "the plan's **current** revision — `published` or
`retired`" so the retired plan keeps a resolvable delta for its in-flight subscribers.

### H-4. A compiled `carry` allowance grant cannot survive a supersession of its source row — the allowance silently stops being issued

`inst-ac-carry` is explicit that the linkage is by row: *"Billing issues, per subscription and
period, **only** the compiled grant whose source row is the subscription's bound
`(currency, region)` row"*, with `source_price_id` named as more than lineage.

But `pricing_price` is attached to `plan_id`, **not** to a revision — S2 §6 says so in as many
words ("the earlier spelling named a `plan_revision` column `pricing_price` does not have") —
so a supersession mints a new `price_id` **without** opening a plan revision (`inst-su-compose`,
`inst-mr-apply`). Meanwhile `pricing_plan_grant` is keyed `(grant_id, plan_revision)` and, per
D-106 plus Foundation §3.7's extended trigger discipline, its rows are **physically immutable
once their revision publishes** ("child rows are physically immutable once their revision
publishes"; the permitted UPDATEs are only `lifecycle_state` flips).

So the successor's publish commit can neither re-point the existing compiled grant's
`source_price_id` nor insert a replacement row under the current published revision. After any
routine reprice of an allowance-carrying usage row, the compiled grant references the
**superseded predecessor**, Billing's "source row = the subscription's bound row" test never
matches, and the included allowance **silently stops being granted** — zero price-row delta,
no validation failure, no alarm. The same structural block hits every allowance-carrying row in
a mass-repricing run, since S12 is built on supersession units.

**Fix shape**: key the compiled grant to the row's **canonical scope key** rather than its
`price_id` (the key is stable across supersession by construction — that is what supersession
*is*), or require an allowance-carrying supersession to open a plan revision, or carve
`source = compiled_allowance` rows out of revision immutability with an explicit recompile step
inside the successor's publish commit. The first is the smallest and matches how every other
cross-supersession reference in the set is written (`inst-cmp-override-home`/D-116 binds the
key family, not the id).

## [M] Findings

### M-1. The allowance compile has no preserved input, so it is not idempotent and blocks reprice

`inst-ac-band` rewrites the row it runs on **destructively**: it prepends `[0, N) @ $0` and
*offsets every authored band bound by `+N`*, and on an untiered row rewrites `model_kind`
`per_unit → graduated` with `amount_minor → NULL`. What it retains is the **authored kind** and
the declaration — *not* the authored band set. `pricing_price_tier_band` holds one band set per
`price_id` (S3 §6), which is now the compiled one.

`inst-ac-deterministic`'s promise — *"the compile is a pure function of the authored
declaration + the row … re-publish recompiles identically"* — therefore has no stored input,
and every re-entry path is undefined. A supersession or repricing successor built from the
published row carries already-offset bands **plus** the declaration, so the compiler's own gate
fires (`ALLOWANCE_DOUBLE_FREE`, `inst-tb-first`) and the reprice is blocked; drop the
declaration instead and the row keeps its price but loses the D-45 marker — the display
("includes N units") and the included-vs-billed reporting split that D-45 exists for. Clone
(`inst-cl-copy`) has the identical ambiguity, and its reset list (`inst-cl-resets`) never
mentions compiled artifacts.

Severity is [M] rather than [H] only because the failure is a blocked publish, not a silent
mis-price. **Fix shape**: persist the authored band set beside the declaration (or flag
compiler-generated bands so the compiler strips and recompiles), and state the round-trip in
`inst-ac-*` plus the clone reset list.

### M-2. The D-79 lane returns one count over a price-id set; every predicate that consumes it is per scope key

PRD §9.2 inbound lane 3: *"Subscriptions reports the **count** of non-terminal subscriptions
whose pinned `pricingSnapshotRef` references **any of a submitted price-id set**"*.

Its consumers all need per-key answers: D-51 keeps or cancels each scheduled window **per key**
(`inst-rt-cancel`), the D-62/D-80 exemption is evaluated **per key** (`inst-fg-trailing`), and
`inst-sg-conjunction` walks keys. A single count over the union cannot decide any of them —
answered as a union it collapses to "does this plan have any subscriber at all", under which
retirement cancels nothing whenever one key is occupied, inverting D-51's point.

The alternative implementation is worse: one call per key means **N synchronous cross-gear
calls inside an ACID transaction** — both `inst-rt-cancel` and `inst-fg-trailing` say the
predicate is "re-resolved **inside** the mutating commit" — with N = keys × markets (a phased
hybrid across 30 markets is in the hundreds), holding the price/window row locks and the
per-tenant audit chain head (M-6) for the whole fan-out.

**Fix shape**: the lane returns a **per-price-id presence map** for the submitted set (one
round trip, N answers), and the design states the timeout and fail-closed budget for a call
made inside a commit. Joint with Subscriptions (their SUB-P8 read is already owed).

### M-3. The two market-uniformity rules range over immutable grandfathered rows, so one cutover permanently freezes tax basis and the proration contract for that market

D-110 `inst-td-basis-uniform`: *"**every published row** of a plan on one `(currency, region)`
MUST carry the same `tax_inclusive` value"*. D-123 `inst-pi-uniform`: *"**every published
recurring row** of a plan on one `(currency, region)`"* must agree on `billingAnchorPolicy`,
`prorationBasis` and `creditOnDowngrade`.

An `existing_grandfathered` generation is a published row on that market; it is immutable in
price and content, MUST NOT be superseded (Foundation §4.3), and never leaves `published` —
expiry is read-derived (`inst-gs-expire`), not a lifecycle flip. So the moment a cutover
happens, those four fields can never change again on that market: any later publish fails
`TAX_BASIS_MIXED_MARKET` / `PRORATION_CONTRACT_MIXED_MARKET` naming a divergent row **nobody
can fix**.

The set is otherwise consistent about this: every sibling row-set rule carves grandfathered
generations out explicitly — `inst-bc-coverage`, `inst-sg-conjunction`, `inst-sg-bundle`,
`inst-mp-grandfathered`, `inst-cl-resets`. These two, the newest of the family, do not.

**Fix shape**: scope both to `priceEligibility ∈ {all_subscriptions, new_subscriptions_only}`.
A grandfathered subscriber's display basis and cycle clock come from its own frozen snapshot,
so the invoice-coherence argument ("an invoice is one document", "a subscription is one cycle
clock") is unaffected by excluding them.

### M-4. `overlay_index` is one per-tenant document, copied on every overlay commit and retained 7 years

D-112 has each overlay publish unit re-project a single `overlay_index` subject row carrying
*"the tenant's live overlay id set with each overlay's scope, interval and precedence"*, and
`pricing_read_model` deltas are retained *"on the same horizon as the append-only truth history
(≥ 7y)"* (Foundation §3.7).

D-112's own accounting — "each overlay commit writes two delta rows … still O(publish units)" —
counts **rows** and not **bytes**. Each index row is O(live overlay count). A tenant with
1,000 live overlays and 10,000 overlay/membership commits a year stores ~10M duplicated index
entries a year, and every single commit rewrites the entire document. It is simultaneously the
**order-time hot object** (one read at the pin, inside the p95 < 100 ms budget) with no cap on
its size — the plan subject got the D-121 horizon precisely for this reason; the index got no
analogue. (The 2026-07-31c perf note already suspected this; the *retention × copy-on-write*
multiplication is the part that had not been computed.)

**Fix shape**: shard the index by `scope_class` (or `(scope_class, scope_value)`) so a commit
rewrites one shard and a lookup reads only the matching ones; bound it the way D-121 bounds the
plan delta (drop overlays whose interval ended before `now − H`); and/or cap live overlays per
tenant with a publish warning, as ADR-0002 does for generations.

### M-5. D-124's aggregate pass assumes all-or-nothing application, but a repricing run's per-row commits may fail individually

D-124 (decided **2026-08-01**) moved the plan-level aggregate pass to run **once**, at the run's
entry to `committing`, *"over the plan's full row set as it will stand post-commit"*.

But `inst-mr-apply` explicitly allows a per-row validation failure to mark that row `failed`
while its siblings apply, and `pricing_repricing_journal` makes partial application a
first-class outcome (*"failed rows are listed on the run report and are retryable only via a
corrected new run"*). So the actual post-commit row set can differ from the set the pass
evaluated, and **nothing re-checks it**: a run that would violate per-market completeness,
phase coverage or meter injectivity under its *actual* outcome commits anyway. This is the
stale-verdict shape D-124 was written to remove, displaced from the approval window into the
commit window.

**Fix shape**: make a plan's rows all-or-nothing within a run — the pass already fails a plan
wholesale (`inst-mr-validate-scope`: "marks **all** of that plan's rows `failed`"), so extend
that to "any per-row failure fails the plan" — or re-run the aggregate pass at the end of each
plan's row set and roll that plan back on failure.

### M-6. The per-tenant audit hash chain serialises every mutation of a tenant, and no NFR accounts for it

D-14 plus the 2026-07-31 per-tenant scoping: the audit row is hash-chained **and committed
inside the mutation transaction** (`inst-au-tamper`, S5 §10). A hash chain is a strict
sequence — writing row *N* needs row *N−1*'s hash — so every audited mutation of a tenant
contends on the same chain head. All authoring on that tenant serialises, by construction.

S12's ratified ≥ 50 rows/s repricing figure (O3) is a per-row-transaction throughput, and its
own per-row cost enumeration (§10: *"row-local rules + the touched key's window
overlap/gap/trailing-void check + 2 window writes + row + outbox + journal"*) **omits the audit
row** — the one write in the list that cannot run concurrently. Interactive authoring contends
on the same head for the duration of the run; so does the M-2 in-commit lane call, which holds
the head across a network round trip.

**Fix shape**: segment the chain (per subject, per day, or an explicit `(tenant, chain_id)`)
with the segment set itself anchored, or keep per-row monotonic sequence numbers and
batch-anchor a Merkle root asynchronously. Either way, name the shape in the O3 sizing and add
the audit write to the per-row cost enumeration.

### M-7. Pin-eligibility is a predicate with no storage, no access path and no publisher

D-101 + D-114 define pin-eligibility as *"committed, **every** subject row of that version
warm-complete, **and** every earlier version itself pin-eligible"*, and the ≤ 5s pin-lag rule
refers to "the newest pin-eligible version" — the frontier's edge.

Nothing stores it. `pricing_read_model` carries a per-row `warm_completed`;
`pricing_catalog_version_ref` carries pending-vs-committed; there is no frontier watermark, no
component owns advancing it, and **no API returns it** — yet every consumer must obtain it
before the first read of a rating run (Foundation §3.6, PRD §9.2 Tariffs "Compatibility"), and
`pricing.readmodel.pin_eligibility_overdue` has to evaluate it to fire. Computed literally, it
is a recursive scan over every subject row of every version since the last known-good one, on a
path budgeted at p95 < 100 ms.

This is the D-112 defect one level up: a resolution rule stated on the truth side with no
access path on the read side — the standing question D-68/D-99 added to the register ("name the
publish unit that makes the fact visible **and** the surface that reads it").

**Fix shape**: materialise a per-tenant monotonic `pin_frontier` watermark advanced by the warm
completer in the same transaction that sets the last outstanding `warm_completed`, expose it on
the read-model contract (that is the value consumers pin), and drive the alarm off it.

### M-8. After D-118, bulk import's approval machinery describes a plane it no longer touches

D-118 pinned bulk import to the draft plane: *"import rows land as **draft** rows — new scope
keys, or edits of existing draft rows under their ETags"*, with published-row changes routed to
repricing runs.

Materiality, however, is evaluated at the submit of a **publish** (`inst-ap-materiality`,
`inst-mat-*` compute deltas against a published baseline), and a draft edit produces no
consumer-visible delta. So `inst-bi-governed` ("a **material** batch routes through the Slice 5
policy before any commit"), the `awaiting_approval` state (`inst-bs-approval`),
`inst-bk-approval-subset`'s per-row content-hash pin, and the D-35 rule that a submitted batch
*"counts as the pending approval unit for **every contained scope key**"* are all residue from
the pre-D-118 reading. The last one is not merely dead: a draft-plane batch pins **published**
keys it cannot change and returns 409 `PENDING_CHANGE_UNIT_EXISTS` to interactive
supersessions on them.

**Fix shape**: state that a draft-plane import is never material — retire `awaiting_approval`
and the D-35 key pin for imports, keeping both for repricing runs where they belong — or state
explicitly which imports still publish and scope the machinery to exactly those.

### M-9. `fixed` is an authorable adjustment kind whose evaluation semantics exist in no document

Pricing authors `adjustment_kind ∈ {markup, discount, fixed}` per overlay line, forces `fixed`
to `magnitude_kind = amount`, and bounds it only by `≥ 0` — D-67's range rules cover `discount`
(`0 < v ≤ 10000` bp) and `markup` (`v > 0` bp) and say nothing about what `fixed` *does*.
Evaluation is Tariffs' by design — and rating's step-4 design enumerates exactly two line
behaviours (`design/04-overlays-precedence.md` §285): *"A percentage line (basis points) scales
the amount; a per-currency absolute amount line applies its published value"*. The kind `fixed`
appears nowhere in the rating gear.

In a stack-all sequential model (`inst-plv-class-tiebreak`, adopted as rating SEAMS O3) the
difference is not cosmetic: "set this line to X" and "add X" produce different amounts **and**
different results under reordering, and rating's `CompositionCapGuard` (`maxCumulativeMarkup`,
rating PRD §527) is defined over *cumulative markup/discount* — a replace-semantics line has no
cumulative interpretation at all, so the anti-drift cap silently does not bind it.

This is the D-01/D-113 class between gears: a field pricing publishes and a consumer is
expected to evaluate, defined by neither. **Fix shape**: state `fixed`'s semantics normatively
in `inst-plv-adjustment` (recommendation: *replace the post-model line amount*, since "markup /
discount / fixed" reads as relative/relative/absolute), state its interaction with the cap, and
register the rating-side adoption.

## Cleanup

- **C-1. A `sum_of_parts` bundle plan has no price rows, and no rule exempts it from the
  row-based plan rules.** `inst-bb-own` states that Slices 3/4 rules apply to `own_price`
  bundles; the `sum_of_parts` counterpart is unstated, so `inst-cs-recurring`/`inst-cs-onetime`
  (≥ 1 base row per sold market for the declared `billing_cycle`), D-15 phase coverage, the
  row-borne descriptor elements (`billingTiming`/`taxCategory`, D-48/D-110) and
  `inst-wc-required` window coverage all apply to a plan that has nothing to satisfy them with.
  Read literally, such a bundle cannot publish under any `billing_cycle` value. §4 delegates
  the lifecycle to "Slices 2/11" without scoping which shape rules come with it.
- **C-2. "The longest billing cycle sold on **the key**" is undefined for keys that carry no
  frequency.** The phrase carries the D-80 coverage horizon (`inst-sg-surface`), the D-04
  copy-window bound (`inst-co-bounds`) and the trailing-void floor (`inst-fg-trailing`) — but
  `usage`, `one_time` and `one_time_setup` rows have no `frequency`, and a one-time plan has no
  cycle at all. D-121 uses the plan-scoped variant ("the longest cycle sold on **the plan**")
  for `H`. Pick one and state the degenerate case.
- **C-3. A required add-on may `depends_on` an optional one, which escapes the coverage
  check.** `inst-cb-addon` case (i) is explicitly scoped to **required** add-ons and
  price-override targets; `inst-cmp-addons` allows arbitrary `depends_on` edges within the
  plan's add-on set. A required add-on depending on an optional one makes the optional one
  transitively mandatory at order time while its per-`(currency, region)` coverage was never
  validated — the D-95 asymmetry through the dependency door, ending at the same order-assembly
  failure. Either evaluate case (i) over the **dependency closure** of the required set, or
  forbid required → optional edges.
- **C-4. The terminal phase's `kind` is pinned only parenthetically.** `inst-ph-graph` says
  "exactly one terminal phase (`evergreen`, no successor)" while the column allows
  `trial | intro | evergreen` and terminality is defined structurally
  (`converts_to_phase_id IS NULL`); no rule or error code enforces the kind. A `trial`-terminal
  plan makes "the first non-trial phase" undefined for both setup timing
  (`inst-cs-setup-timing`) and migration entry (D-39), and collides with
  `CHECK (display_trial_days = phase_duration_days)` since duration is forbidden on the
  terminal phase. An `intro`-terminal plan ("intro pricing forever") is plausible authoring and
  is neither allowed nor rejected.
- **C-5. The `migrated-origin` payload materialises row content only.** D-87/`inst-sy-payload`
  freezes per-row content (model kind, bands/package, evaluation-policy + S6 contract fields,
  tax basis, rounding policy). The **plan-level** billing descriptor set (invoice line
  template, GL code, itemization rule) and the entitlement grant set are not in it — and by
  construction resolvable nowhere else, since the ref pins no `CatalogVersion`. Billing cannot
  post a legacy line from the payload alone, and for a tier-2 (fully legacy) key there may be
  no plan revision to fall back to.
- **C-6. D-125's page cap and the export SLO unit are unreconciled.** The contract sets
  `limit` default 100 / hard cap 1,000 and says the p95 ≤ 5s **/ 100 records** SLO applies "per
  page/chunk". Read linearly that makes a full page a 50s response — outside any reasonable
  gateway timeout. Either scale the SLO with the page size explicitly or lower the cap to the
  SLO's unit.
- **C-7. Clone's copy/remap set is incomplete.** `inst-cl-copy` remaps the `phase` axis of
  copied price rows but not the D-41 `entitlement_grants.perPhase` map, which is keyed by
  `phase_id` (fails closed at publish with `GRANT_SET_PHASE_UNKNOWN` — visible but unexplained);
  and the copy set says nothing about `pricing_plan_grant` / `pricing_composite_meter` (S10) or
  the compiled allowance artifacts (M-1), which are silent.
- **C-8. D-113's routing does not name `PlanLink` migration.** `inst-pc-counter-carry` routes
  the target plan's `usageCounterOnPlanChange` "at an **in-place** plan change"; a `PlanLink`
  migration is the other in-place plan move and carries its own contract (S11
  `inst-mg-boundary` covers setup charging and entry phase, not the counter). The default
  `reset` plus the unit-match gate make the unsafe case unreachable, so this is completeness of
  the routing statement rather than a live hole.

## Checked and refuted

Recorded so the next pass does not re-derive them.

- **The 20-currency floor vs the 500-rows-per-plan soft cap** (carried forward unverified from
  2026-07-31d). Recomputed on a deliberately hostile shape — 4 phases × 60 `(currency, region)`
  markets of recurring coverage (240) + 3 phase-invariant meter lines × 60 markets (180) + a
  setup row per market (60) = **480 rows** — which sits inside a cap that is explicitly *soft*
  **Corrected 2026-08-05 (verification pass): this arithmetic is not the worst case and must not be cited as the bound.** It omits `priceEligibility` and `cohort` — the two axes ADR-0002 added *because* they multiply rows — and retained generations are `published` rows on distinct keys, so they land inside the candidate row set and are counted; one cutover per recurring key roughly doubles the figure. The refutation still holds, but it rests on **D-160's advisory status** (the cap never blocks: the finding rides `warnings[]` and `is_publishable()` does not read it) rather than on any row count. The 180-row term is separately unauthorable in this crate — see **D-196**.
  and tenant-configurable, with usage rows phase-invariant by design (`inst-ph-usage-invariant`)
  keeping the phase axis off the largest term. **Not a finding**; the carried item can be
  closed.
- **`overlay_index` size** (carried forward unverified from 2026-07-31d) — **upheld and
  sharpened**, see M-4: the live-set bound was the smaller half; retention × copy-on-write is
  the actual cost.
- **The cutover copy colliding with the scope-key partial `UNIQUE`.** The copy lands on a new
  `cohort` key, so D-100's predecessor flip covers the only real collision; the copy needs no
  flip of its own.
- **`ALLOWANCE_DOUBLE_FREE` as a silent mis-pricing path.** It blocks the publish rather than
  mis-pricing — which is why M-1 is [M] and not [H].
- **The materiality baseline being read from the read model rather than from truth** (S5 §10).
  A lagging read model yields an *older* baseline, hence a *larger* delta, hence a more
  conservative materiality verdict. The error direction is safe; no finding.

## Verification & fix record

Fix wave 2026-08-01, on the owner's go; every fix applied against the cited document text.

| Finding | Decision | Fix | Where it landed |
|---------|----------|-----|-----------------|
| H-1 (first generation unbindable) | **D-126** · veto-flagged · joint w/ Rating | `cohort = none` ⇒ the generation whose `cohort` = the pinned row's window `effectiveTo`; no such generation ⇒ the class contributes no candidate and resolution continues down the class order | ADR-0002 (decision bullet + Confirmation test); S7 `inst-el-bootstrap` (new) + `dod-grandfathering` + AC ×3; S1 §4.1 |
| H-2 (cutover successor unguarded) | **D-127** | the guard binds the **key**, not the mechanism — both producers set `supersedes_price_id` and pass it; rule renamed the *succession* unit guard | S3 `inst-tb-supersession-units` + §5 + `dod-tier-bands` + AC; S7 `inst-co-successor` + `dod-grandfathering` + AC; S1 §4.3 ×2; PRD §17.5 |
| H-3 (retirement invisible at the pin) | **D-128** | retirement becomes a publish unit; `lifecycle_state` a projected field; projector sources the **current** revision (`published` or `retired`); partial `UNIQUE` widened | S1 §4.2 (new) + §4.4 + §3.7 ×2; S11 `inst-rt-event` + `dod-retirement` + AC; S2 `inst-pl-retire` + `inst-pl-supersede`; S7 `inst-sg-surface`; PRD `fr-plan-retirement` + §17.5 table (new row) |
| H-4 (carry grant stranded by reprice) | **D-129** | grant binds `source_scope_key`, `source_price_id` demoted to lineage; `included_allowance` joins the preserved set on `carry` rows | S10 `inst-ac-carry` + §6 + `dod-included-allowance` + AC; S3 `inst-tb-supersession-units` + §5 + `dod-tier-bands` + AC; S1 §4.3 |
| M-1 (compile destroys its input) | **D-130** | the compile is a **projection**; truth keeps the authored declaration/kind/amount/bands; supersedes D-59's rewrite mechanism, keeps its `volume` ban | S10 `inst-ac-band` + `inst-ac-deterministic` + §6 + DoD + AC ×2; S3 Q6 + `inst-tb-first` + §6; S12 `inst-cl-copy` + `dod-clone` |
| M-2 (lane shape vs per-key predicates) | **D-131** · joint w/ Subscriptions | per-price-id **presence map**, one call per mutating unit over the union, stated timeout | PRD §9.2 lane 3; S7 `inst-fg-trailing`; S11 `inst-rt-cancel` + AC |
| M-3 (uniformity freezes a market) | **D-132** · veto-flagged | both rules scoped to `priceEligibility ∈ {all_subscriptions, new_subscriptions_only}` | S4 `inst-td-basis-uniform` + DoD + AC; S6 `inst-pi-uniform` + §6 + DoD + AC; S8 `inst-bc-taxbasis`; PRD ×2 FRs |
| M-4 (`overlay_index` cost) | **D-133** | `subject_ref = (scope_class, scope_value)` + D-121 horizon; ≤ 6 point reads, one shard per commit | S9 §7 + §10 + AC; S1 §3.7 |
| M-5 (aggregate pass vs partial apply) | **D-134** · amends D-111/D-124 | the run commits **per plan**, pass inside that transaction, any row failure fails the plan | S12 `inst-mr-apply` + `inst-mr-validate-scope` + state machine ×2 + DoD + §10 + AC |
| M-6 (audit chain serialises a tenant) | **D-135** · amends D-14 | chains segmented per `(tenant, chain_id)` + a per-tenant roll-up; audit joins the per-row cost model | S5 G4 + `inst-au-tamper` + `dod-audit` + AC; S1 §3.7; S12 §10 |
| M-7 (frontier has no storage/surface) | **D-136** · completes D-101/D-114 | materialized `pricing_pin_frontier` watermark + `GET /v1/pricing/catalog-version/frontier` | S1 §4.4 + §3.3 + §3.7 (new table); S5 AuthZ endpoint map; PRD §9.2 |
| M-8 (import approval residue) | **D-137** · completes D-118 | an import is never material and pins no key; `awaiting_approval` + the hash pin stay with repricing | S12 `inst-bi-governed` + `inst-bk-approval-subset` + state machine ×2 + §4 + DoD + AC |
| M-9 (`fixed` undefined) | **D-138** · veto-flagged · joint w/ Rating | `markup` adds, `discount` subtracts, **`fixed` replaces**; resets the cumulative-markup accumulator; `FIXED_LINE_DISCARDS_STACK` warning | S9 `inst-plv-adjustment` + §5 + §6 + DoD + AC; PRD `fr-priceoverlay-authoring` |
| C-1 (row-less bundle) | — | `inst-bb-rowless`: which plan rules a `sum_of_parts` bundle answers to | S8 `inst-bb-rowless` (new) + `dod-bundle-composition` |
| C-2 (cycle-on-key undefined) | — | W6 defines the term plan-scoped, zero on a plan with no recurring part | S7 §1.6 W6 (new) + `inst-sg-surface` |
| C-3 (dependency closure) | — | case (i) evaluates the `depends_on` closure of the required set | S4 `inst-cb-addon` + AC |
| C-4 (terminal phase kind) | — | terminal `kind` MUST be `evergreen`; `TERMINAL_PHASE_KIND_INVALID` | S2 `inst-ph-graph` + §5 |
| C-5 (payload lacks plan-level content) | — | descriptor set + resolved grant set join the frozen payload | S11 `inst-sy-payload` + §6 + `dod-snapshot-synthesis` + AC; PRD §9.2 |
| C-6 (cap vs SLO unit) | — | SLO is per 100-record chunk, scaling to the cap; full pages are export shapes | S12 `inst-he-export` + `dod-history-export`; PRD `fr-price-history-export` |
| C-7 (clone copy/remap set) | — | `perPhase` keys remapped; grants/composites copied; compiled grants recompiled | S12 `inst-cl-copy` + `dod-clone` |
| C-8 (D-113 vs migration) | — | the routing names `PlanLink` migration explicitly | S6 `inst-pc-counter-carry` |

## Coverage — what this pass did not cover

Stated plainly so the next reviewer starts from the right place.

- **PRD requirement substance** was read only where a finding depended on it (§9.2, §17.5,
  §6.6). The AC corpus (~120 entries) was not walked.
- **Cross-gear**: rating was read for grandfathering selection (H-1), overlay evaluation (M-9)
  and the composition cap; subscriptions only through pricing's §9.2 lanes. No systematic
  census.
- **Four of the six items 2026-07-31d carried forward unverified remain unverified**: the D-114
  frontier as a publishing freeze (M-7 addresses its *mechanism*, not its liveness cost), the
  24h idempotency TTL vs an approval lifetime with no expiry, `pricing_operator_flag` having no
  read endpoint, and the GDPR-vs-WORM disposition for payer membership history.
- **Frontend/Studio, fixtures repo content, and the gear skeleton** are out of scope for a
  design-set review.
