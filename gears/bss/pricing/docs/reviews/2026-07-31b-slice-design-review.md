<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — manual slice review (2026-07-31, second pass of the day)

**Scope**: all 12 slice designs ([`../design/`](../design/)) + [`DESIGN.md`](../DESIGN.md) +
[`DECISIONS.md`](../DECISIONS.md) (D-01…D-98, §F.1/§F.2) + targeted PRD verification.
**Method**: single-reviewer sequential pass, slice by slice, no subagents; every candidate
finding cross-checked against the decision register and the PRD text before being claimed.
Mechanical gate: `spec-check` over pricing with the sibling gears loaded — **0 findings**
(the 14 reported pricing-only are `seam-undefined` / `propagation-target-not-loaded`
artifacts of a single-gear corpus, none of them real).
**Baseline**: everything already registered — the open product forks (§F.1), the carried
findings (§F.2), the D-87…D-98 wave and its veto confirmations — is deliberately **not**
re-reported.

**Verdict**: the semantic layer is clean where the last six waves looked; this pass finds a
different failure surface — **the read side**. Five waves hardened the truth side (append-only
rows, revision rows, atomic units, instant floors) and the register's own doctrine says
"nothing becomes consumer-visible outside a committed `CatalogVersion`" — but three
consumer-visible mechanisms never got a publish unit or a stable resolution rule: window
mutations (H-1), a pinned version's own resolution (H-3), and the `migrated-origin` payload
(H-4). Alongside them the cutover — the one atomic unit D-88 did *not* rewrite — cannot commit
as written (H-2). The rest is the familiar pattern one door further along: a decision landed on
its cited surface and not on the adjacent one (M-2 bundles vs D-50, M-4/M-5 revisions vs
D-83/D-92, M-6 the contract lock vs D-56).
Totals: **4 [H]**, **10 [M]**, **7 [L]**.

> **Status (verification pass, same session): ALL FINDINGS CONFIRMED AND FIXED.** Every finding
> was independently re-verified against the cited document text before its fix was applied — none
> was rejected or downgraded. H-1…H-4 and M-1…M-10 are closed as **D-99…D-112** in
> [`DECISIONS.md`](../DECISIONS.md); the [L] items are text fixes in place. The per-finding mapping
> is in the [Verification & fix record](#verification--fix-record). `spec-check` re-run after the
> wave: **clean** (pricing, sibling gears loaded). **Awaiting veto: D-103, D-104, D-108, D-109,
> D-110** — the five that change an authorable surface or add an approval requirement.

Severity scale: **[H]** breaks money/correctness or is unimplementable as written · **[M]**
teams can build incompatible behavior · **[L]** contained.

---

## [H] Findings

### H-1. A window mutation is consumer-visible and is not a publish unit — the read side never learns

The PRD's `CatalogVersion` increment table is explicit: *"Price-only edit (amount/**window** on
existing plan) → Yes — content MUST become addressable in a `CatalogVersion`"* ([`../PRD.md`](../PRD.md)
§17.5). The design implements addressability in exactly one place: it is requested **on
`PlanPublished`** (`fr-catalogversion-increment`; [`01-foundation.md`](../design/01-foundation.md)
§4.2 step 5), and D-06 extended the set of publish units to overlays and memberships — and
stopped. `pricing_read_model.subject_kind ∈ {plan, price_overlay, group_membership}`: there is
no window subject, so window facts can only ride the **plan** subject, which only a plan
publish re-projects. Yet S7's standalone `WindowScheduler` surface — `POST /prices/{id}/windows`,
`PATCH /price-windows/{id}`, `DELETE /price-windows/{id}` — requests nothing, warms nothing and
emits only `PriceWindow*` events, while the gate's predicates are required to be *"point-in-time
evaluable from the pinned read model"* including the per-key **coverage end**
([`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md) `inst-sg-surface`/`inst-sg-pinned`;
PRD `fr-sellability-gate`), and Tariffs step 2 resolves the active window per key from published
content. Three consequences, all money:

- **Cancelling a scheduled window leaves the projection advertising coverage.** The truth side
  runs `inst-fg-trailing` and rejects the dangerous cases; the exempted ones (and every
  legitimate cancel) leave the last-warmed delta unchanged, so the gate keeps selling into
  exactly the trailing void D-62 → D-80 → D-94 spent three decisions closing. `inst-fg-when`'s
  *"there is no side door … a gap can never be introduced past validation"* is true of
  `pricing_price_window` and false of what consumers read.
- **Remediation is invisible.** An operator extending coverage to lift a D-80 horizon block
  changes nothing until some unrelated publish re-projects the plan — the key stays unsellable
  with no operator-reachable path, which is the failure mode D-85 rejected option (b) over
  ("unbounded lag waiting for an unrelated publish").
- **Rating resolves a window state the truth side has already changed** (shortened `effectiveTo`,
  cancelled successor).

Fix direction: make every `WindowScheduler` mutation a publish unit on the plan subject
(validation → pending ref → warm), exactly the D-06 treatment — the doctrine-consistent option;
the alternative, moving the window/coverage predicates out of the pinned read model onto a live
surface, contradicts `inst-sg-pinned` and the p95 < 100ms order-time budget.

### H-2. The cutover cannot commit: nothing flips the predecessor, and only the supersession unit may

`inst-co-successor` schedules the `all_subscriptions` successor on the **same** canonical scope
key as the shortened predecessor (same `planId`/currency/region/overlay/phase, `priceEligibility
= all_subscriptions`, `cohort = none`, same `chargeKind` — only the grandfathered *copy* moves to
a new key). The Foundation admits one published row per key
(§3.7 partial `UNIQUE … WHERE lifecycle_state = 'published'`). But `inst-gc-commit` enumerates
the whole transaction — *"shorten `effectiveTo` + two window schedules + the two new rows"* — and
never flips the predecessor; and both the price-row state machine and the orchestrator name the
D-88 unit as the sole path: *"the **only** path to `published → superseded`"*
([`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md) §1.7 `SupersessionOrchestrator`),
*"there is no primitive-by-primitive path to this flip"*
([`03-price-structure.md`](../design/03-price-structure.md) `inst-ps-supersede`). Only S1's trigger
whitelist remembers the case (`published → superseded` **"on supersession/cutover"**).

Assembled from the documented rules the cutover's commit inserts a second published row on an
occupied key and dies on the unique index — the atomic unit fails wholesale at commit, and the
grandfathering mechanism ADR-0002 exists for never runs. This is H-2 of the previous wave with
the operands swapped: D-88 defined the supersession unit and left its multi-row sibling holding
a rule that forbids what the sibling must do.

Fix: state the predecessor flip in `algo-cutover` (step 3/4, inside the one ACID transaction) and
widen `inst-ps-supersede` to name the cutover as the second sanctioned path — or compose the
cutover's `all_subscriptions` half **as** a supersession unit and keep the flip in one place.

### H-3. A pinned `CatalogVersion` does not resolve stably — per-subject fallback plus an unbounded degraded warm

D-91 moved the warm-completion marker from the version to the subject row, and §4.4 states the
partial-batch semantics plainly: subjects *"resolve independently as their warms complete, each
falling back to its own prior completed delta meanwhile"*. Resolution is *"the subject's greatest
completed version ≤ the pin"*. Therefore **one pin resolves two different contents over time**:
a consumer pinning `V` reads plan `P` at `V-3` before `P`'s delta warms and at `V` after — with no
version ever mutating, so the monotonicity argument §4.4 offers does not address it. The
neighbouring clause loses its referent entirely: *"at pin time the pinned version MUST NOT lag
the newest completed version by more than 5s"* — "completed version" was a version-level property
before D-91, and nothing now defines when a version becomes **pin-eligible**.

The exposure is not the 5s SLO. After a degraded publish the warm re-drive *"continues past the
SLO"* (§3.6/§4.4) with no stated bound, and S1 §1.2 is explicit that the batching-delay SLO
governs the pre-commit delay *"not … degraded handling"* — so the divergence window is
operationally unbounded. Two concrete failures:

- **Replay divergence (money).** A rating run pinning `V` charges the pre-change price for a plan
  whose delta has not warmed and the post-change price for one that has; re-running the same
  pinned run later yields different money — the reproducibility `fr-pricing-snapshot` promises
  in the one place it still resolves.
- **Silent old price past a changeover.** At the changeover instant the truth side shortens the
  predecessor's window and activates the successor's (`inst-ws-activate` fires on wall clock
  alone). A consumer whose pin has not warmed resolves the *predecessor* row **with its
  pre-shorten, open-ended window** and keeps charging the old price — not fail-closed, silently
  wrong. D-88's instant floor bounds only the batching lag, by construction.

Fix direction: define pin-eligibility (either a version is pinnable only once every subject of
its publish batch is warm — restoring version-level completion, which is what the ≤ 5s clause
assumes — or a subject that has not warmed at the pinned version **fails closed** rather than
falling back), and add the missing conjunction guard: a window whose changeover arrives while its
row's subject is not warm raises Critical and holds activation rather than switching into an
unresolvable state.

### H-4. The `migrated-origin` payload has no read surface — D-87's consumption gap, one level out

D-87 made the synthesized snapshot self-contained precisely so rating could charge from it, and
the PRD states the obligation: *"Rating/Tariffs evaluate **from that payload** without resolving
its ids through the read model or any `CatalogVersion`"* (`fr-migration-safety`);
`inst-sy-firstrating` has rating *"retry against the frozen result"*. Nothing exposes it. The
payload lives in `pricing_snapshot_provenance` ([`11-lifecycle.md`](../design/11-lifecycle.md) §6),
which is deliberately outside `pricing_read_model` — S1 §4.4 names `migrated-origin` *"the one
deliberately non-version-pinned reference"* — and:

- S11 §5's API surface has no provenance/snapshot read endpoint;
- S5's endpoint → `(resource, action)` map, which claims to cover *"every REST surface declared
  by Slices 2–12"*, has none either;
- none of the five §9.2 integration contracts carries it — `contract-tariffs-readmodel`
  enumerates snapshot-frozen **read-model** fields only.

Compounding it, this is also the one place the catalog composes a **per-subscription** snapshot,
which two other normative statements forbid: D-30 — *"the catalog publishes membership into the
read model; **Tariffs** resolves … and freezes the group into the snapshot **it** composes … no
catalog resolve-for-payer endpoint"* — and S9's own DoD, *"the catalog never stamps snapshots,
per the Foundation composition rule"*. So the single artifact the catalog does compose
per-subscription is the single artifact with no hand-off, no owner-side surface and no contract
lane. Verbatim the D-01/D-87 class: the rule is written, the consumption mechanism does not exist.

Fix: name the surface and register it as a §9.2 lane — a service-identity read
(`GET /v1/pricing/migrated-origin-snapshots/{subscriptionRef}`, `plan × read`) and/or delivery on
the `:start` response and the completion record — and reconcile the composition-ownership
sentence so D-30 and `SnapshotSynthesizer` describe one model.

---

## [M] Findings

### M-1. "Exactly one `meteringUnit` per plan revision" vs three rules that price several meters
`fr-meter-injective` and its §17 mirror are unambiguous — *"Each usage plan **revision** MUST map
**exactly one** `meteringUnit`… Multi-meter offerings MUST be modeled as a derived (composite)
meter or as separate single-meter SKUs"* — and S2 `inst-cmp-injective` restates it. Three rules
assume the opposite: D-84's per-line completeness rules over *"every `(meter, dimensionKey)` line
the plan prices"*; S2's own integration AC exercises *"a usage-only plan pricing meter M1 in two
markets and M2 in one"*, a plan `METER_AMBIGUOUS` already rejects — so the AC is unreachable and
`USAGE_MARKET_INCOMPLETE` never fires for the reason given; and D-43's `inst-pg-applicability`
scopes a grant to *"an explicit **set** of published `meteringUnit` ids … usage lines of the
grant-bearing plan"*, a set that can only ever hold one element. Two validators contradict, and
the resolution changes what each rule means: either injectivity is per-`(meter, dimensionKey)`
**line** only (and `METER_AMBIGUOUS` + the composite-meter rationale are wrong) or it is one
meter per plan (and D-84's scope collapses to dimension keys while `GRANT_APPLICABILITY_*` loses
its purpose).

### M-2. Bundle composition and rev-share changes carry no materiality trigger — D-50's hole, one slice over
The `MaterialityEvaluator` computes per-row deltas over **price rows**, and `inst-mat-registered`
enumerates every always-material trigger — bundles appear nowhere. A `sum_of_parts` component
swap, a rev-share re-split, or an `invoiceItemization` flip produces **no** price-row delta, so
with a threshold configured it evaluates `auto_publishable` and reaches consumers with **no
approver** — while a $1 price-row change above threshold takes two people. This is exactly the
hole D-50 closed for overlay lines ("an overlay is not a price row… the G1 no-delta rule applies
wholesale"), and the money is comparable: a rev-share split is vendor payout, a component swap
changes what the customer receives at an unchanged price. It also voids D-11's own justification
for dropping `bundle × write` from the publish endpoint — *"the composition is protected at
publish time by the approval content pin"* — because a non-material publish opens no approval
record and therefore no pin. (`FinanceReviewer` already holds `bundle × read`, so the D-61
reviewability invariant costs nothing here.)

### M-3. Three child tables are keyed so they can hold only one row per revision
D-83/D-92's "keyed `(X, plan_revision)`" phrasing dropped the row discriminators on every 1:N
child table except phases. `pricing_plan_addon_rule` is *"keyed by `(plan_id, plan_revision)`"*
yet holds one row per `addon_sku_id`; `pricing_bundle_component` *"keyed `(bundle_id,
plan_revision)`"* holds one per component; `pricing_bundle_revshare` the same while holding one
per `(vendor_sku_id, party)`; `pricing_bundle_revshare_group` states in prose *"one row per
`vendor_sku_id` within a revision"* and then omits `vendor_sku_id` from its key. Under the keys
as written, the `depends_on` cycle walk has one edge to walk, *"every referenced component"* is
one component, and *"sum to 100% **per** included vendor SKU"* is unsatisfiable. `pricing_plan_phase`
shows the correct shape (`(phase_id, plan_revision)`). Related: `pricing_bundle` declares no
`plan_id` at all, so D-92's *"a bundle rides its plan's revisions"* has no join path.

### M-4. D-83/D-92's revision discipline still misses two plan child tables
`pricing_plan_grant` (PK `grant_id`, FK `plan_id`) and `pricing_composite_meter` (PK
`composite_id`, FK `plan_id`, plus an unexplained bare `revision` column) carry no
`plan_revision` — and both hold structural, snapshot-frozen content: the grant's
`category`/`applicability`/`drawdownPriority`/prices, and the composite formula whose own
constraint A4 says *"versioned with the plan revision"*. So a draft revision editing a grant's
applicability or a composite formula mutates the **published** revision's truth, and a
degraded-warm re-drive can leak the draft into a frozen version. That is the D-83 finding
verbatim, third occurrence: D-83 closed S2's three tables, D-92 closed S8's and S9's, and the
two remaining plan children were never swept.

### M-5. The overlay `precedence` uniqueness and interval-overlap checks are not revision-scoped
D-92 keyed `pricing_price_overlay` `(price_overlay_id, revision)` with `draft | published |
superseded` rows coexisting, but left `UNIQUE (tenant_id, scope_class, precedence)` unqualified —
so opening a draft revision of a published overlay collides with **itself** and every edit fails
`PRECEDENCE_DUPLICATE`. The same applies to `OVERLAY_INTERVAL_OVERLAP`, whose collision key
`(scope_class, scope_value, planId, targetSku, cohort)` will always match the overlay's own
published revision. Both the price-row index and the plan-revision index got the partial
`WHERE lifecycle_state = 'published'` treatment (D-90 leans on it explicitly); the overlay
indexes did not travel with the decision that created the revisions.

### M-6. The contract lock is keyed to a plan revision, and a revision owns no price
`inst-cl-reject` and PRD `fr-contract-locked-protection` both read: while an active contract
references a plan **revision**, *"structural plan mutation MUST be rejected, directing the
operator to a new Plan revision or contract expiry"*. Under D-56 a published revision row is
immutable in content by construction (§4.3) — so the guard rejects something already impossible
— and `pricing_price` was deliberately kept attached to `plan_id`, **not** to a revision. A lock
on revision N therefore protects no price: a supersession or repricing run on the plan's keys, a
cutover, or an overlay line targeting the plan all move a locked subscription's economics at its
next renewal, and none of them is enumerated or blocked. D-78 established that "the row is
immutable and the effective charge moved anyway" is a real defect class, not a technicality.
Needs either the enumeration (locked ⇒ these named operations reject) or an explicit statement
that Contracts holds the rate and this guard is documentation only.

### M-7. Retirement is a one-person, irreversible operation that cancels windows
D-62 made a single window cancel/shorten **always material** because one operator could
otherwise *"silently revert a two-person-approved price change"*. Retirement cancels **every**
not-yet-active window on its zero-subscriber keys in one call under `plan × retire`, and
`inst-mat-registered` registers it as material **only** when a live cutover unit exists. There is
no `retired → published` edge (`inst-pl-norollback`), so the act is irreversible, and the dry-run
confirm screen is the only control on a mutation that stops all new sales for the plan. Either
plain retirement joins the always-material triggers (`plan × read` is already held by the
approver, so the D-61 invariant is satisfied), or the asymmetry with D-62 is stated as
deliberate.

### M-8. A per-plan descriptor `tax_category` is declared to mirror a per-row `tax_category_ref`
S4 §6: *"`tax_category_ref` is the **source of truth** for a **row's** tax category: the D-48
billing-descriptor set's tax-category field mirrors it, with a publish-time consistency check — a
mismatch fails publish"*. But `pricing_plan_descriptor_set` holds exactly **one** `tax_category`
per `(plan_id, plan_revision)`, while a plan legitimately carries different categories per row
(subscription vs data transfer) and per region. The consistency check is undefined the moment two
rows differ, and as written it silently forces one tax category per plan — which no rule states
and which the per-row column exists to avoid. Adjacent and unstated: nothing constrains
`tax_inclusive` uniformity across the keys of one `(currency, region)`, so a market can publish a
tax-inclusive recurring line beside a tax-exclusive usage line. Pre-Tax-Engine the D-94
conjunction merely makes that market unsellable (one flagged key blocks the plan-market, with no
publish-time explanation to the operator); post-GA it is a mixed-basis invoice.

### M-9. Performance: the D-88 unit puts a whole-plan validation inside every per-row transaction of a repricing run
`inst-su-commit` has the supersession unit's commit *"re-run the pipeline"*; `inst-mr-apply` has
*"each applied row … a supersession unit … executed inside the per-row transaction"*. The
pipeline is the **aggregate** rule set — D-21 puts window coverage, phase coverage, hybrid
completeness, meter injectivity and the fixture gate at publish only — so a run over N rows of
one plan performs N whole-plan validations (plus 2 window writes) inside row-level transactions,
against a ratified ≥ 50 rows/s. S12 §10 sizes only the window writes (*"their throughput is part
of this slice's own O3 sizing"*); the pipeline re-run arrived with D-88, after the figure was
perf-verified. Fix direction: state that a bulk run's per-row commit re-runs the **row-local**
rule set plus the key's gap/overlap check, with the plan-level aggregate pass run **once per plan
per run** (it evaluates identical content for every row of that plan) — or re-derive O3 and
re-verify.

### M-10. Overlay resolution has no access path from a plan to its overlays
D-91 projects one row per overlay subject and forbids re-projecting targeted plans: *"Tariffs
joins overlays to base rows at evaluation"*. To evaluate, Tariffs must enumerate every
scope-matching overlay at its pin — but the delta store is keyed `(tenant_id, catalog_version,
subject_kind, subject_ref)` with the read index `(tenant_id, subject_kind, subject_ref,
catalog_version DESC)`, so there is no path from a plan / brand / region / customer group to the
overlay set. The only route the documents leave is a `DISTINCT subject_ref` scan across the
retained overlay deltas (retention = the truth-history horizon, ≥ 7y) followed by a
greatest-≤-pin probe per subject — on the order-time path with a p95 < 100ms budget. S9 §10 sizes
membership resolution only (*"an indexed interval lookup per `(payer, group)`"*) and says nothing
about overlay enumeration. Needs a scope-indexed current-overlay projection, or an overlay-index
subject per version, named where D-91 named the rest.

---

## [L] Findings

- **L-1.** `pricing_plan.lifecycle_state` is still enumerated `draft | published | retired` in S1
  §3.7, in §3.1's `Plan` bullet and in §3.2's `DraftStateMachine`, while D-90 added
  `superseded` (S2 §4 carries all four). The same paragraph that omits it then describes the
  `published → superseded` flip.
- **L-2.** `tier_qualification_window` (D-40, re-homed by D-60) is declared in **no** data-model
  table — it occurs exactly once in the design set, inside `dod-trailing-tier`'s *Touches* list;
  neither S3's nor S10's `pricing_price` column table has it. `TierQualificationValidator` is
  likewise absent from S10 §1.7's design-introduced names.
- **L-3.** The bulk lock is offered as *"a marker on `pricing_price` rows (`bulk_operation_id`
  nullable column or a lock side-table, implementation choice)"* — but the append-only
  column-whitelist trigger permits exactly two UPDATEs on a published row (`lifecycle_state`
  flips, `grandfather_until` tightening), so the column option is illegal on precisely the rows
  a repricing run locks. Only the side-table is a real choice.
- **L-4.** The PRD §17 reference row for the sellability gate still reads *"all **six**
  predicates … hold for **the bound canonical scope key** at `t`"* (singular) — D-94's
  conjunction reached `fr-sellability-gate` and §9.2 but not the §17 table.
- **L-5.** S6's `crossBoundaryChangePolicy` / `crossBoundaryWarningText` are *"projected once per
  contract version"* into `pricing_read_model`, which since D-91 has no tenant- or
  contract-level subject (`subject_kind ∈ {plan, price_overlay, group_membership}`). Either they
  ride every plan subject or the subject taxonomy needs the fourth kind.
- **L-6.** S2 `inst-cs-usage` still says *"`tierAggregationWindow` **when tiered**"*, excluding the
  `package` case D-58 made mandatory and D-70 propagated everywhere else; S3's
  `EVAL_POLICY_MISSING` code description repeats the omission (*"unset on a tiered usage row"*)
  while `inst-pk-window` raises that very code for a `package` row.
- **L-7.** A material membership mutation (a bulk group move, `inst-mm-bulk`) has no pre-commit
  storage. `inst-as-reject` says a rejected non-plan subject returns to *"its slice-defined
  pre-submit state (… membership change not applied …)"*, and S7 explicitly parks the cutover's
  three-operation payload in `pricing_approval` — S9 defines no holding place for a pending
  membership change set, so the approval record pins a content hash over content that lives
  nowhere. (The *immediate* re-resolution half of this is inside the §F.1 fork; the bulk move is
  not.)

---

## Performance assessment

The publish path, the activation job and the repricing journal remain well shaped; the read path
is where this pass finds cost, and it is the same place the correctness findings land.

- **The order-time hot path is no longer a single indexed read** as §10 claims in three slices.
  Under D-91 an evaluation needs: one probe for the plan subject, one per membership subject, and
  an **unbounded enumeration** of overlay subjects with no scope index (M-10) — inside p95 <
  100ms. This is the D-86 explosion avoided on the write side and reintroduced on the read side.
- **The repricing SLO is now sized against the wrong unit of work** (M-9): D-88 put a full
  aggregate pipeline run inside each per-row transaction, so the run is O(rows × plan-validation
  cost), not O(rows). The ratified ≥ 50 rows/s predates that change.
- **The degraded-warm window is unbounded and load-bearing** (H-3). Every determinism claim that
  survives — replay, "posted periods never re-query", the ≤ 5s pin-lag rule — assumes a version
  becomes resolvable atomically. Per-subject markers traded that for warm parallelism without
  pricing the trade.
- **Window mutation propagation (H-1), once fixed as a publish unit, adds one `CatalogVersion`
  request + one plan re-projection per window operation.** That is the correct cost, but it must
  be counted: the cutover writes 3 window ops, a supersession 2, and a repricing run 2N — the
  D-47 interactive ≤ 5s coalescing and bulk ≤ 5 min max are what keep it affordable, and the
  bulk path already coalesces.
- Unchanged and still sound: publish-path validation is off the rating path; the activation job
  scans by index under a lease; the journal commits row + outbox + journal in one transaction;
  the D-79 lane is synchronous and fail-closed but bounded per mutation.

## Per-slice verdicts

| Slice | Verdict |
|-------|---------|
| 01-foundation | H-1 and H-3 are rooted here (publish-unit set, subject-typed markers); L-1 |
| 02-plan-definition | M-1 (co-owner), M-3 (add-on rules), L-6; the phase machinery is tight post D-64/D-89 |
| 03-price-structure | H-2 (co-owner via `inst-ps-supersede`), L-2, L-6; D-77/D-82/D-98 themselves landed accurately |
| 04-currency-tax | M-8; otherwise clean (D-95 landed cleanly) |
| 05-governance | M-2 and M-7 are gaps in `inst-mat-registered`; the AuthZ catalog and D-61 invariant remain the most mature surface in the set |
| 06-consumer-contracts | L-5; D-93's read-time move landed accurately |
| 07-pricewindow-linkage | H-1, H-2, M-7 (co-owner); the D-88 supersession unit itself is well specified |
| 08-bundles | M-2, M-3; L-7-adjacent nothing — the slice is otherwise consistent |
| 09-price-overlays | M-5, M-10, L-7; D-42/D-78/D-91's line semantics are clean |
| 10-advanced-primitives | M-1 (applicability), M-4, L-2 |
| 11-lifecycle | H-4, M-6, M-7; the two-tier synthesis selection (D-76/D-81/D-87) is otherwise complete |
| 12-operator-efficiency | M-9, L-3; the journal/lease/abort machinery is sound |

---

## Verification & fix record

Second pass, same session: every finding re-verified against the cited document text before any
fix was applied. **All 4 [H] + 10 [M] + 7 [L] CONFIRMED**; none rejected, none downgraded.

Verification notes worth keeping. For **H-1** the PRD decided it: its increment table already
said a **window** edit must become addressable in a `CatalogVersion`, so the design was in
violation of its own requirements document, not merely under-specified — which also settled the
fix direction, since the alternative (move the coverage predicates onto a live surface)
contradicts `inst-sg-pinned` and the order-time budget. For **H-2** the three-way contradiction
was decisive and mechanical: same scope key + one-published-row-per-key + a state machine naming
the *other* unit as the sole flip path = a commit that cannot succeed; S1's trigger whitelist
already listed "supersession/**cutover**", so the fix restores a rule the schema layer never
lost. For **H-3** the check was whether pin-eligibility could stay per-subject: it cannot without
either fail-closing every consumer during a warm lag or keeping the divergence, so version-level
pin-eligibility with per-subject *storage* is the only option that preserves both D-91's
parallel warms and replay determinism. For **H-4** the endpoint map was the tell — S5 claims to
map "every REST surface declared by Slices 2–12", so the payload's absence there was proof rather
than suspicion. For **M-1** the enforcing index arbitrated: it carries `meter` **and**
`dimension_key`, so the per-line reading was already implemented and only the prose said
otherwise. For **M-6** the two halves were checked separately — that a published revision is
immutable by §4.3 (so the guard had no referent) and that `pricing_price` hangs off `plan_id`
(so a revision owns no price) — before concluding the mechanism, not the wording, had to change.

| Finding | Verdict | Fix | Where it landed |
|---------|---------|-----|-----------------|
| H-1 (window mutations not publish units) | **CONFIRMED** | **D-99** — schedule/adjust/cancel are publish units re-projecting the plan subject; the read model carries window **intervals**, so activation/expiry stay projection-free | S7 `inst-ws-publishunit` (new) + `inst-sg-surface` + §5 + DoD + AC + §10; S1 §4.2 + §4.4; PRD §17.5 + `contract-tariffs-readmodel` |
| H-2 (cutover cannot commit) | **CONFIRMED** | **D-100** — the cutover commit flips the predecessor; it is the second sanctioned producer of `published → superseded` | S7 `inst-co-supersede` (new) + `inst-gc-commit` + DoD + AC; S3 `inst-ps-supersede`; S1 §4.3 ×2 |
| H-3 (pin resolves unstably) | **CONFIRMED** | **D-101** — version-level pin-eligibility (per-subject storage unchanged) + `pin_eligibility_overdue` / `changeover_unwarmed` Criticals | S1 §1.2 + §3.6 + §4.4; S7 `inst-ws-changeover-warm` (new) + §7 + AC; PRD `fr-consumer-readmodel-resolution` + `contract-tariffs-readmodel`; the D-91 entry amended |
| H-4 (`migrated-origin` unreadable) | **CONFIRMED** | **D-102** — `GET /v1/pricing/migrated-origin-snapshots/{subscriptionRef}` (`plan × read`, service identity) + a §9.2 lane; D-30 narrowed to its actual case | S11 `inst-sy-surface` (new) + §1.7 + §5 + DoD + AC; S1 §4.4; S5 endpoint map; PRD `fr-migration-safety` + `contract-tariffs-readmodel`; the D-30 entry narrowed |
| M-1 (injectivity vs multi-meter) | **CONFIRMED** | **D-103** — per `(meter, dimensionKey)` line per scope-key slice; a plan MAY price several meters; **flagged for veto** | PRD `fr-meter-injective` + §17 row; S2 `inst-cmp-injective` + `dod-composition` + AC ×2 |
| M-2 (bundle materiality) | **CONFIRMED** | **D-104** — composition/rev-share/basis/itemization changes are always-material; **flagged for veto** | S8 `inst-ba-material` (new) + DoD + AC; S5 `inst-mat-registered` + DoD + AC; PRD `fr-bundle-composition` |
| M-3 (child-table keys) | **CONFIRMED** | **D-105** — discriminators restored on 4 keys; `pricing_bundle.plan_id` added | S2 §6 + AC; S8 §6 ×4 + AC |
| M-4 (grant/composite revisions) | **CONFIRMED** | **D-106** — `plan_revision` on both, identity halves stable; `pricing_grant_price` re-keyed | S10 §6 ×3 + AC |
| M-5 (overlay indexes) | **CONFIRMED** | **D-107** — precedence index partial on published; overlap check excludes sibling revisions | S9 §6 + `inst-plv-dating` + AC |
| M-6 (contract lock subject) | **CONFIRMED** | **D-108** — the lock is structural; price movement is Contracts-owned, stated normatively; **flagged for veto** | S11 `inst-cl-scope` (new) + DoD + AC; PRD `fr-contract-locked-protection` + `fr-plan-retirement` |
| M-7 (retirement one-person) | **CONFIRMED** | **D-109** — always material unconditionally; **flagged for veto** | S11 `inst-re-governed` + DoD + AC; S5 `inst-mat-registered` + DoD + AC; PRD `fr-plan-retirement` |
| M-8 (tax category cardinality) | **CONFIRMED** | **D-110** — `taxCategory` rides the row (descriptor column removed); `TAX_BASIS_MIXED_MARKET`; **flagged for veto** | S4 ×5 sites; S2 P5 + `inst-ds-required` + §6 + DoD; PRD ×4 sites; the D-48 entry revised |
| M-9 (bulk validation cost) | **CONFIRMED** | **D-111** — row-local per row + the key's window checks; plan-level aggregate once per plan per run | S12 `inst-mr-validate-scope` (new) + DoD + §10 + AC ×2; the D-88 entry amended |
| M-10 (overlay access path) | **CONFIRMED** | **D-112** — a tenant `overlay_index` subject per version | S9 §7 + §10 + AC; S1 §3.7 + §4.4; the D-91 entry extended |
| L-1…L-7 | **CONFIRMED** | Text fixes in place (see the [L] list) | S1 §3.1/§3.2/§3.7; S2 `inst-cs-usage`; S3 §5 code; S6 §6 + AC; S9 `inst-mm-pending` (new); S10 §6 + §1.7; S12 §6 + AC; PRD §17 gate row |

## What this wave says about the method

The last two waves' shared pattern was "a decision landed on its cited surface but not on the
adjacent one", and the standing heuristic — *for each new decision, ask which sibling surface
carries the same shape* — caught M-3, M-4, M-5 and M-7 here. What it did **not** catch is the
[H] set, because those are not propagation gaps: H-1, H-3 and H-4 are all the same unasked
question — **"which side reads this, and through what?"** Every one of them is a mechanism that
was completed on the truth side and never given a consumption path, which is the D-01 class the
register has now closed five separate times (D-01, D-65, D-79, D-87, and now these). A third
standing question is worth adding beside the sibling-surface one:

> For every consumer-visible fact, name the publish unit that makes it visible and the surface
> that reads it. If either is absent, the rule is not finished.
