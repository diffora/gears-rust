<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — manual slice review (2026-07-30)

**Scope**: all 12 slice designs ([`../design/`](../design/)) + [`DESIGN.md`](../DESIGN.md) +
[`DECISIONS.md`](../DECISIONS.md) (D-01…D-78, §F.1/§F.2) + targeted PRD verification.
**Method**: single-reviewer sequential pass, slice by slice, no subagents; every candidate
finding cross-checked against the decision register and the PRD before being claimed.
**Baseline**: everything already registered — open product forks (§F.1), carried findings
(§F.2), veto-flagged decisions — is deliberately **not** re-reported. Only new findings below.

**Verdict**: the set is mature — four review waves have cleaned out most defect classes.
Remaining: **3 [H]** (two are the D-01 disease pattern — a rule with no data source; one is a
money defect on supersession), **4 [M]**, and a tail of [L]. Performance thinking is solid
except one systemic gap (read-model per-version growth/retention).

> **Status (verification pass, same session): ALL FINDINGS CONFIRMED AND FIXED.** Every
> finding was independently re-verified against the cited documents before fixing — none was
> rejected or downgraded. H-1…H-3, M-1…M-4 and the performance gap are closed as
> **D-79…D-86** in [`DECISIONS.md`](../DECISIONS.md); the [L] items are text fixes in place.
> The per-finding mapping is in the [Verification & fix record](#verification--fix-record)
> at the end of this document. **Veto round 2026-07-31**: D-79/D-80/D-81/D-83 — and the
> older flagged backlog D-56/D-58/D-59/D-60(+D-69)/D-73/D-74 — **CONFIRMED per-item** by the
> product owner; the register carries the statuses.

Severity scale: **[H]** breaks money/correctness or is unimplementable as written · **[M]**
teams can build incompatible behavior · **[L]** contained.

---

## [H] Findings

### H-1. "In-flight subscribers per scope key" has no data source — and D-51/D-62 rest on it

Retirement keeps continuing-coverage windows "only for scope keys with **no in-flight
subscribers**" ([`11-lifecycle.md`](../design/11-lifecycle.md) `inst-rt-cancel`), the D-62
exemption for window cancel/shorten evaluates the same predicate — including for
**materiality routing** ([`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md)
`inst-fg-trailing`; [`05-governance.md`](../design/05-governance.md) `inst-mat-registered`) —
and the retirement confirm screen must label kept vs cancelled windows per key.

But pricing owns no subscription data by design, and the Subscriptions contract
([`../PRD.md`](../PRD.md) `cpt-cf-bss-pricing-contract-subscriptions`) declares exactly
**two** inbound lanes: grandfathering re-bind feedback and the migration execution handshake
(D-65). Neither supplies a per-scope-key subscriber count. The PRD user story "View the
active subscription count" (retirement flow, step 1) hangs on the same missing input.

This is exactly the defect class D-01 and D-65 were closed on: the rule is written, the input
does not exist. **Fix**: a third inbound lane in PRD §9.2 (per-scope-key active-subscription
read or feed), with an explicit staleness caveat — the value can change between read and
commit (see M-1, which compounds this).

### H-2. Snapshot synthesis: tier 2 is unreachable — three rules are mutually unsatisfiable

- Import rejects any reference row whose effective dates "reach `now` or later" or whose
  range intersects a not-yet-closed billing period
  ([`05-governance.md`](../design/05-governance.md) `inst-bd-noeffect`).
- Synthesis freezes state "as of the **trigger instant**" — effectively now (M4,
  [`11-lifecycle.md`](../design/11-lifecycle.md) `inst-sy-freeze`; PRD `fr-migration-safety`).
- Tier 2 of D-76's selection rule picks the reference row whose
  `[effective_from, effective_to)` **covers `t`** (`inst-sy-select`).

Together: no legally imported row can ever cover the synthesis instant — tier 2 never fires.
D-13's stated purpose ("backdated rows shape `migrated-origin` snapshots that rating consumes
**going forward**") is unconstructible under these constraints: a legacy plan no longer sold
fails closed forever (its reference rows are all strictly past), and a plan still sold
resolves through tier 1 at the **current** price — not the legacy price synthesis exists to
reconstruct. D-76 fixed the structural half (disjoint store); the temporal half survived it.

**Fix**: decide one of — (a) the synthesis `t` may be historical (and define which instant:
subscription inception, rated-period start, …), or (b) `inst-bd-noeffect` gets a carve-out
for intervals extending into the future, with an explicit side-effect analysis. Either way,
re-state `inst-sy-select`'s `t` and add an AC that exercises tier 2 end-to-end.

### H-3. `Q` counter continuity across supersession is unprotected against unit-changing successors

`inst-tb-window-continuity` ([`03-price-structure.md`](../design/03-price-structure.md)):
the tier counter lives on `(subscription, meter, dimensionKey, window)`, supersession does
**not** reset it, "the new row's bands are simply applied to the continued `Q`". But no rule
forbids the successor changing `billingGranularity`, `aggregationFunction` /
`aggregationGranularity`, or the `meter` itself — supersession constraints are only "same
canonical scope key, one eligibility class, one `chargeKind`"
([`01-foundation.md`](../design/01-foundation.md) §4.3), and none of these fields is a
scope-key axis.

A mid-window supersession `per_hour` → `per_day`: the continued `Q` counted in hours is
applied to bands denominated in days — the same ×24 band-edge error D-77 just closed for
level rows, reintroduced through supersession. (A `meter` change silently *resets* the
counter by construction — different counter key — which is at least self-consistent but
stated nowhere.)

**Fix**: a publish check on supersession — a successor changing unit-determining fields
(`billingGranularity`, `aggregationFunction`, `aggregationGranularity`, `meter`,
`tierAggregationWindow` where the counter is live) either fails publish or is explicitly
defined as a counter reset; plus a scenario in the supersession-continuity fixture.

---

## [M] Findings

### M-1. D-51/D-62 exemption race: "no in-flight subscribers" is evaluated at cancel time, but selling continues

The sellability gate is six point-in-time predicates — **future coverage is not one of them**
([`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md) `inst-sg-surface`).
Scenario: key with active window W1 (ends t2) and approved scheduled successor W2; zero
subscribers → cancelling W2 passes with no trailing-void check and no approval (the
exemption). The plan stays **sellable until t2**; anyone who subscribes in [cancel, t2) lands
in the trailing void at t2 — arrears and renewals fail closed. The exact hazard class D-62
closed, reopened through its own exemption.

**Fix options**: evaluate the exemption as "no in-flight subscribers **and** the key is not
currently sellable", or add a sellability predicate "covered through now + longest billing
cycle sold on the key". Note this also needs H-1's data source.

### M-2. The draft-revision model (D-56) does not extend to the plan's child tables

`pricing_plan` got `(plan_id, revision)`; phase rows "re-attach with stable ids"
([`02-plan-definition.md`](../design/02-plan-definition.md) §6). But a phase row is a
**single row per `phase_id`** (mutated on re-attach), and `pricing_plan_addon_rule` /
`pricing_plan_descriptor_set` have no revision dimension at all. Consequences:

1. Editing draft revision N+1 mutates the truth-side state of published revision N — two
   variants of composition/descriptors have nowhere to coexist.
2. A degraded-warm **re-drive** ([`01-foundation.md`](../design/01-foundation.md) §4.4) can
   re-project revision N with N+1's draft edits — "consumers never read draft" violated
   through a side door — and the **projection source for re-warm (truth tables vs a
   publish-time frozen payload) is specified nowhere**.

**Fix**: either version child rows with the revision (copy-on-new-revision), or make the
projector normatively read a publish-time snapshot, never live tables.

### M-3. A hybrid's usage part is not required to exist per sold market — "sold but unrateable" through a coverage hole

Per-market completeness is required for one-time and recurring rows
([`02-plan-definition.md`](../design/02-plan-definition.md) `inst-cs-onetime`/
`inst-cs-recurring`), phase coverage is recurring-only (D-74), hybrid completeness is "≥ 1
usage row **anywhere**" (`inst-cs-hybrid`). No rule anywhere requires usage rows per market
(PRD checked: the phase-glossary coverage is recurring-only too; bundles, by contrast, get
explicit per-market component coverage — a telling asymmetry). Sellability evaluates the keys
that exist — an absent usage key fails nothing.

Result: a hybrid with recurring in EUR+USD and usage only in EUR is sellable in USD, and the
USD subscriber's usage events fail closed — the state D-15/D-17 declare "impossible by
construction". **Fix**: publish check — on a hybrid, the usage part covers every market where
a recurring row exists (or an explicit authored "usage not sold in market X" fact).

### M-4. Drift flags vs the frozen read model — the projection mechanism is undefined

`tier_divergent` ([`02-plan-definition.md`](../design/02-plan-definition.md)
`inst-cmp-tier-drift`), `grants_divergent`
([`06-consumer-contracts.md`](../design/06-consumer-contracts.md) `inst-gs-drift`), and
`readiness_divergent` (S4) must flag published plans "**in the read model**" on an external
registry/Tax-Engine signal. But the read model is monotonic per `CatalogVersion` and advances
only through publish units — a flag has no publish unit. This is the exact problem D-06
solved for overlays/memberships; the flags did not receive the same treatment. Either the
flag mutates a frozen version in place (doctrine violation — a pinned consumer sees the
version change mid-run) or it waits for an unrelated publish (unbounded lag).

**Fix**: declare drift flags operator-plane state outside the versioned read model, or make
the drift signal its own publish unit per the D-06 pattern.

---

## [L] Findings

- **L-1.** [`01-foundation.md`](../design/01-foundation.md) §3.7: the partial-UNIQUE
  predicate "`published` **and not superseded, via the supersession link**" is not
  expressible as a partial-index predicate (the link lives on the successor row) — and after
  the flip-at-commit fix it is redundant (`lifecycle_state = 'published'` suffices). Same
  section: "a published predecessor and its scheduled successor legally coexist" contradicts
  §4.3 (the predecessor is `superseded` at commit). Wording cleanup.
- **L-2.** [`01-foundation.md`](../design/01-foundation.md) §3.7: "published revision rows
  are immutable" — retire flips `lifecycle_state` on exactly such a row; D-56's own text is
  more careful ("in-place mutation survives only as state-machine flips"). Wording.
- **L-3.** [`03-price-structure.md`](../design/03-price-structure.md) `inst-la-fields`:
  `aggregationGranularity` present on a `sum` row is not forbidden (for `maxHold` it
  explicitly is). An accepted-but-ignored value — against the set's own doctrine that D-77(c)
  cites.
- **L-4.** [`07-pricewindow-linkage.md`](../design/07-pricewindow-linkage.md)
  `inst-gc-commit`: the cutover instant must be future at approval commit, but nothing
  requires a margin ≥ the batching/warm lag (D-47: bulk up to 5 min). An instant inside the
  lag → the successor's window is active while its row is not yet resolvable at any completed
  `CatalogVersion` — transient fail-closed for renewals/arrears on the key. Recommend:
  validate instant ≥ commit + SLO margin (or document renewal retry semantics).
- **L-5.** [`09-price-overlays.md`](../design/09-price-overlays.md) `inst-plv-dating`: the
  cross-overlay interval-collision key `(scope_class, scope_value, planId, targetSku)` was
  not extended with `cohort`, while the within-overlay UNIQUE was (D-78). A cohort-targeted
  line and a cohort-less line on the same `(plan, sku)` in two overlays are disjoint by
  eligibility yet rejected as `OVERLAY_INTERVAL_OVERLAP`; inside one overlay the same pair is
  legal. Add `cohort` to the collision key.
- **L-6.** Reservation × `includedAllowance` on one row: not forbidden, not specified — the
  reserved remainder starts `Q` at 0 (`inst-rv-tier-q`), then the compiled `[0, N)` band
  grants the remainder another N free units. Possibly intended; neither a rule nor a fixture
  scenario covers the combination ([`10-advanced-primitives.md`](../design/10-advanced-primitives.md)).
- **L-7.** [`02-plan-definition.md`](../design/02-plan-definition.md)
  `inst-cs-setup-timing`: "a plan change never **re**-charges the target's setup row" — for a
  plan-change entrant who never paid any setup, "re-" is misleading (is the target's setup
  charged at all? Intended reading appears to be: setup charges only at subscription
  activation, regardless of later plans — say it plainly).
- **L-8.** [`08-bundles.md`](../design/08-bundles.md): usage-only components in a
  `sum_of_parts` bundle — neither forbidden nor specified (what sums; how rev-share applies
  to usage revenue); the frequency-match rule covers recurring components only. p2 slice, but
  worth pinning.
- **L-9.** [`11-lifecycle.md`](../design/11-lifecycle.md): the fate of an open draft revision
  row when the plan is retired is unstated (D-56 × retirement).

---

## Performance assessment

Well-considered overall: single indexed version-pinned reads on the read path; sellability on
the order hot path from the pinned read model; membership as an interval lookup at
snapshot/renewal time (not per rating call); band validation O(n log n) with the 100-band
cap; repricing with the `(run_id, price_id)` journal, one-transaction row+outbox+journal, and
version coalescing; the activation job batching over `(state, effective_from)`.

One systemic gap: **read-model per-version storage semantics**
([`01-foundation.md`](../design/01-foundation.md) §3.7, `pricing_read_model` keyed
`(tenant_id, catalog_version, plan_id)`). Unstated whether a version stores a full tenant
copy or a delta with "greatest version ≤ pin" resolution — with ≤ 5s interactive coalescing a
full copy per publish explodes storage; a delta changes the read contract. And **retention of
historical versions is undefined entirely**, while replay/reproducibility require old
versions to stay resolvable for years — unbounded growth either way. This must be decided
before code: it shapes both the schema and the read contract.

## Per-slice verdicts

| Slice | Verdict |
|-------|---------|
| 01-foundation | Core coherent; M-2/M-4 and the retention gap are rooted here; wording L-1/L-2 |
| 02-plan-definition | M-2, M-3, L-7 |
| 03-price-structure | H-3, L-3 |
| 04-currency-tax | Clean (open tails already in §F.2) |
| 05-governance | H-1 (materiality routed through a sourceless predicate); otherwise the most mature slice in the set |
| 06-consumer-contracts | M-4 (`grants_divergent`); otherwise clean |
| 07-pricewindow-linkage | M-1, L-4; the window machinery itself is tight after D-62/D-63 |
| 08-bundles | L-8 |
| 09-price-overlays | L-5; D-42/D-78 landed accurately |
| 10-advanced-primitives | L-6; D-59/D-60 propagated correctly |
| 11-lifecycle | H-1, H-2, L-9 |
| 12-operator-efficiency | Clean; the bulk × approval × D-35 interplay is consistent |

## Recommended register actions

- **H-1, M-1** → new DECISIONS.md entries, **joint with Subscriptions** (a third §9.2 inbound
  lane + the exemption-evaluation semantics).
- **H-2** → a DECISIONS.md entry re-stating the synthesis `t` semantics vs the import
  constraints (S5 × S11).
- **H-3** → a DECISIONS.md entry + S3 publish rule + a supersession-continuity fixture
  scenario.
- **M-2, M-4** → design fixes in Foundation/S2 (child-table revisioning or frozen projection
  source; drift-flag plane).
- **M-3** → a publish rule in S2/S4.
- **[L] items** → text fixes in place; L-5 is a one-line key change.

---

## Verification & fix record

Second pass, same session (completed 2026-07-31 00:xx CEST): every finding re-verified
against the cited document text before any fix was applied. **All 16 findings + the
performance gap CONFIRMED**; none rejected. Fixes applied per the register actions above.

Verification notes worth keeping: for **H-2**, PRD AC (§ Migration safety, "two
implementations MUST freeze identical prices") turned out to already define the per-trigger
instant — `migration` → the migration effective timestamp, `first-rating` → the earliest
unrated usage timestamp — which the design's M4 had flattened to "the trigger instant"; both
instants are unreachable by tier 2 under the old import bound (the first is future-of-
scheduling under the D-49 notice floor, the second sits inside an open billing period by
definition), so the fix went to the **import side** (option b), not the instant. For **L-5**,
the asymmetry was confirmed exactly as claimed: the within-overlay `UNIQUE` carries `cohort`
(S9 §6), the cross-overlay collision key did not. For **M-1**, "not currently sellable" alone
was found insufficient (an unsellable key can become sellable again — `availableFrom`
arriving, a GA gate clearing — with the void still scheduled), which is why the fix pairs the
narrowed exemption with the predicate-(1) coverage horizon and the reconciliation alarm.

| Finding | Verdict | Fix | Where it landed |
|---------|---------|-----|-----------------|
| H-1 (subscriber predicate has no source) | **CONFIRMED** | **D-79** — third §9.2 inbound lane (per-scope-key presence over price ids), fail-closed on outage, re-resolved in-commit; joint w/ Subscriptions, flagged for veto | PRD §9.2; S7 `inst-fg-trailing`; S11 `inst-rt-cancel` + §10; S5 `inst-mat-registered` |
| H-2 (synthesis tier 2 unreachable) | **CONFIRMED** | **D-81** — per-trigger `t` restated design-side; import allows `effective_to` ≥ now / open-ended (`effective_from` stays strictly past); open-period-intersection rejection dropped (D-76 store is structurally inert); flagged for veto | S5 `inst-bd-noeffect` + §6 + DoD + AC; S11 M4 + `inst-sy-freeze` + `inst-sy-select` + DoD + AC (tier-2 e2e); PRD `fr-migration-safety` + `fr-historical-import-governance` |
| H-3 (`Q` continuity vs unit-changing successor) | **CONFIRMED** | **D-82** — `SUPERSESSION_UNIT_MISMATCH` publish check: successor preserves `meter`/`dimensionKey`/granularities/aggregation windows; unit changes route via revisioning + migration; negative fixture scenario | S3 `inst-tb-supersession-units` (new) + §5 + DoD + fixture + AC; S1 §4.3; PRD `fr-supersession` + §17.5 |
| M-1 (exemption races the gate) | **CONFIRMED** | **D-80** — exemption narrowed (+ not-sellable); predicate (1) gains the coverage horizon (`now +` longest cycle sold; count stays six); `pricing.window.coverage_ending_with_subscribers` alarm; joint w/ Subscriptions, flagged for veto | S7 `inst-fg-trailing` + `inst-sg-surface` + §7 + DoD + AC ×2; S5 `inst-mat-registered`; PRD `fr-sellability-gate` + §9.2 |
| M-2 (draft revisions don't cover child tables) | **CONFIRMED** | **D-83** — copy-on-new-revision: `pricing_plan_phase` PK `(phase_id, plan_revision)`, addon-rule/descriptor-set keyed by revision; projector normatively reads the published revision's rows (warm + re-drive); flagged for veto | S1 §3.7 + §4.3 + §4.4; S2 §6 ×3 tables + AC |
| M-3 (hybrid usage not per-market) | **CONFIRMED** | **D-84** — `USAGE_MARKET_INCOMPLETE`: every priced `(meter, dimensionKey)` line in every sold market (hybrid **and** usage-only); `$0` row is the free-market remediation | S2 `inst-cs-hybrid` + `inst-cs-usage` + §5 + DoD + AC; PRD `fr-hybrid-completeness` |
| M-4 (drift flags vs frozen read model) | **CONFIRMED** | **D-85** — operator-plane store `pricing_operator_flag`, never the versioned read model; overlay flags unaffected (they ride their causing publish unit) | S1 §3.7 + §4.4; S2 `inst-cmp-tier-drift` + DoD; S6 `inst-gs-drift`; S4 `inst-td-readiness` |
| Perf (read-model storage/retention) | **CONFIRMED** | **D-86** — per-plan delta, greatest-completed-≤-pin resolution, retention on the truth-history horizon | S1 §3.7 + §4.4 |
| L-1 (partial-UNIQUE predicate; stale "published predecessor" sentence) | **CONFIRMED** | Predicate reduced to `lifecycle_state = 'published'` (flip-at-commit makes it sufficient; the link lives on the successor row anyway); coexistence sentence restated (superseded predecessor + published successor) | S1 §3.7 (`pricing_price`) |
| L-2 ("published revision rows are immutable" vs retire flip) | **CONFIRMED** | "Immutable **in content**; only sanctioned mutation = `lifecycle_state` flip" — D-56's own framing | S1 §3.7 (`pricing_plan`) + §4.3 |
| L-3 (`aggregationGranularity` accepted-but-ignored on `sum`) | **CONFIRMED** | Joins `LEVEL_FIELDS_INVALID` (forbidden like `maxHold`) | S3 `inst-la-fields` + §5 + AC |
| L-4 (cutover instant inside the batching lag) | **CONFIRMED** | Instant ≥ commit + max batching-delay SLO (D-47, 5 min) at approval commit | S7 `inst-gc-compose` + §2 error + §5 code |
| L-5 (collision key missing `cohort`) | **CONFIRMED** | `cohort` added to the cross-overlay interval-collision key (now matches the within-overlay UNIQUE) | S9 `inst-plv-dating` + §5 code |
| L-6 (reservation × `includedAllowance` unspecified) | **CONFIRMED** | Forbidden at publish (`ALLOWANCE_WITH_RESERVATION`) — a named Future gate, D-53 posture | S10 `inst-ac-gate` + §5 code |
| L-7 ("never **re**-charges" misleading) | **CONFIRMED** | "Never charges the target's setup row at all — whether or not the origin carried one; setup is tied to activation" | S2 `inst-cs-setup-timing` + DoD; S11 `inst-mg-boundary`; PRD glossary + `fr-one-time-setup` |
| L-8 (usage-only bundle components unpinned) | **CONFIRMED** | Pinned **legal**: recurring amounts sum, usage rates per component rows; frequency-match recurring-only by construction; rev-share covers the vendor SKU's entire rated revenue | S8 `inst-bb-sum` + `inst-bc-frequency` + `inst-rs-sum` |
| L-9 (open draft revision × retirement unstated) | **CONFIRMED** | Draft revision row deleted (audited) in the retirement transaction; no `retired → draft` edge | S11 `inst-rt-cancel` |
