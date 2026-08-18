<!-- Related: ../DESIGN.md, ../DECISIONS.md, ../design/ | Owners: BSS Product Catalog team -->

# Pricing design set — billing-domain review (2026-07-31, fourth pass of the day)

**Scope**: all 12 slice designs ([`../design/`](../design/)) + [`PRD.md`](../PRD.md) +
[`DECISIONS.md`](../DECISIONS.md) (D-01…D-122) + the three ADRs, read against the consuming
gears' own documents (rating `docs/`, subscriptions `docs/`) through a **billing-domain lens**
(money semantics, governance, cross-gear contract coherence) — a different cut from the four
sequential slice passes.
**Method**: multi-agent — domain-semantics finder lanes + verification agents + a
non-functional lane, every surviving candidate verified against the document text before being
claimed.

> **Coverage — read this before using the findings.** Two of the four verification agents and
> all six domain-semantics finders were **killed mid-run by a session limit**, so their lanes
> never reported. What is below is only what was verified against the document text by hand or
> by a completed verifier. **Not covered**: money semantics of S1–S3 and S4/S5/S8/S9 line by
> line, the cross-gear snapshot-field census, follow-through of the D-99…D-122 fix waves, and
> PRD requirement substance. Six candidates from the non-functional lane (§Unverified below)
> are deliberately **not** reported as findings. `spec-check` over pricing + rating +
> subscriptions is clean for pricing (6 Low findings, all in the neighbour gears' registers).

Verification retracted **3 of 11** checked candidates outright and **downgraded 6 more** from
the severity they were reported at; the tiers below are **post-verification**. Totals:
**1 [H]**, **3 [M]**, **7 Cleanup**, **3 Minor**, 3 retracted, 6 unverified carried forward.

> **Status (fix wave 2026-08-01, on the owner's go): ALL SURVIVING FINDINGS FIXED.**
> H-1 → **D-123**; M-1 + M-2 → **D-124** (one decision, two clauses — same instruction);
> M-3 → **D-125**; C-1…C-7 and N-1…N-3 are text/schema fixes in place. The per-finding mapping
> is in the [Verification & fix record](#verification--fix-record).
> **Veto round 2026-08-01: D-123 CONFIRMED** (with D-113/115/117…119/122 — nothing in pricing
> awaits veto). Owed adoptions: Subscriptions + Rating — **informational only** (their existing
> single-value readings become well-founded; no field or contract change). The six §Unverified
> candidates remain deliberately unactioned pending their own verification pass.

Severity scale: **[H]** breaks money/correctness or is unimplementable as written · **[M]**
teams can build incompatible behavior · **Cleanup/Minor** contained, latent, or hygiene.

---

## [H] Findings

### H-1. `billingAnchorPolicy` is authored per recurring row — N rows per plan-market under the phase axis — while Subscriptions reads one cycle clock per subscription

`billing_anchor_policy` is a **`pricing_price` column** (S6 §6) and `inst-pi-required` makes it
mandatory on every recurring row. D-15 (`inst-ph-coverage`) requires recurring coverage of
every phase per sold `(currency, region)`, and phase is a scope-key axis (ADR-0002, S1 §4.1) —
so a phased plan has N recurring rows per market, each carrying its own anchor. On the consumer
side `billingAnchor` is a **single field on the Subscription aggregate** — "the PRD's
cycle-boundary rule" (subscriptions `design/01-foundation-lifecycle` §aggregate, subscriptions
PRD glossary). Failure: an intro-pricing plan (intro phase → evergreen, both charging) in
EUR/EU authors `subscription_start` on the intro row and `fixed_day(1)` on the terminal row.
Both publish, both sit in one frozen snapshot, and no rule says which one sets the
subscription's cycle boundary — one implementation keeps the original boundary at phase
conversion, another shifts it to the 1st and prorates a partial period. Neither gear's
documents mention anchor × phase interaction at all. Precedent for the fix shape: D-110
`inst-td-basis-uniform` imposed exactly this rule on `tax_inclusive` ("an invoice is one
document"), and usage rows were deliberately made phase-invariant
(`inst-ph-usage-invariant`) to avoid this class — recurring rows got the opposite treatment
with no coherence rule. `prorationBasis` and `creditOnDowngrade` are the same class one notch
weaker: rating splits per `SubWindowSlice` "each with its own snapshot" (rating `design/09`)
and `inst-pi-credit-source` already names the governing row across a change, but subscriptions
reads `prorationBasis` as one value "applied to all mid-period proration of the recurring
component" (subscriptions PRD).

**Fixed as D-123**: the three proration-contract fields are uniform across the recurring rows
of one plan-`(currency, region)` (`PRORATION_CONTRACT_MIXED_MARKET`, 422); per market, not per
plan; `billingTiming` exempt (deliberately per-row). Flagged for veto · joint.

## [M] Findings

### M-1. D-111's soundness argument cites a protection that does not exist in the window it covers

`inst-mr-validate-scope` hoisted the plan-level aggregate pass to run once per plan "inside the
run's approval-to-commit boundary", justified by "an interactive edit on a contained key is
already blocked (`inst-mp-pending`, the bulk lock)". Both citations fail for that window:
`inst-mp-pending` is a **selector-time per-row rejection** of rows whose key already holds a
pending unit — not a block on edits started later — and the bulk lock provably does not exist
yet (`inst-bs-commit`: "the bulk lock takes effect on entry to `committing`";
`inst-bs-approval`: rows "not locked while awaiting"). Since the same decision narrows the
per-row commit to the row-local set plus the touched key's window checks, a stale aggregate
verdict is never re-checked at commit. Stated honestly: **no concrete money-loss scenario was
built** — an interactive plan revision runs its own aggregate pass at its commit (S1 §3.6) and
the per-row unit guard still catches D-82/D-98 mismatches — so this is an unsupported
justification, not a demonstrated hole.

### M-2. The aggregate pass's input — the run's frozen row set — cannot decide plan-level rules

Same instruction, same line: the pass runs "over the run's frozen row set", but the rules it
carries (phase coverage, hybrid/per-market completeness, meter injectivity) are properties of
the plan's **whole** post-run row set. A selector matching only the plan's USD rows yields a
frozen set that cannot evaluate per-market completeness across the plan's other markets at all.

**M-1 + M-2 fixed as D-124** (one decision, two clauses): the pass runs at the run's entry to
`committing` — inside the bulk lock, where the cited protection actually holds — over the
plan's full row set as it will stand post-commit.

### M-3. The unbounded read surfaces have no pagination contract

A broad grep over `PRD.md`, `DESIGN.md` and all 12 slices returns **zero** hits for
`paginat|cursor|$top|$skip`, while S5 `inst-au-read` states "export p95 ≤ 5s / 100 records" —
an SLO expressed **per page with no page contract anywhere**. Affected:
`GET /v1/pricing/history` over 7 years of append-only rows, `GET /v1/pricing/audit`, the plan
lists, the overlay list, batch reports. Not a platform convention the gear may lean on: the
sibling rating gear does declare pagination in its own PRD/SEAMS, and `guidelines/` has no
pagination standard.

**Fixed as D-125**: Foundation-level cursor contract (S1 §3.3), inherited by every slice
surface; the SLO's unit is the page/chunk.

## Cleanup

- **C-1. Tenant-rule cross-references repointed.** S5 §6 and S4 §6 attributed
  "tenant-scoped, SecureORM" to "Foundation §3.7", whose parenthetical is scoped to
  Foundation-owned tables and extends only the **name prefix** to slice tables; S3's fixture
  carve-out cited "Foundation §3.1" (the Domain Model). The rule actually lives in §2.2
  (`cpt-cf-bss-pricing-constraint-authz-gate-fnd`) plus S5 `inst-rb-pep`. All ten slice §6
  headers now carry the uniform citation. Recorded as Cleanup, **not** a security finding: an
  isolation-hole reading was checked and refuted (§2.2 gates every ctx-bearing path before the
  repository, `inst-rb-pep` binds the SecureORM filter to `tenant_id`, every slice inherits
  the Foundation C-set glossed as including tenant isolation, and S3's carve-out is explicit —
  you do not carve out from a habit).
- **C-2. S5's taxonomy enumerations widened to D-120's four classes.** The config-label object
  and the endpoint row still read `taxonomies/{region,brand}` while S4 declares
  `pricing_partner_taxonomy`/`pricing_org_tier_taxonomy` with the same discipline and its DoD
  already used `taxonomies/*`. Text only — the grant exists; the "these endpoints have no
  authz pair" reading was refuted. S4 `dod-taxonomy` Touches gains the two tables.
- **C-3. `inst-co-single-pending` generalized to match its own response gloss.** The rule
  enumerated "(cutover or supersession)" while the §5 gloss was already subject-agnostic ("a
  pending unit already holds one of the touched keys"). After D-62, D-104 and D-109 there are
  always-material units that touch a key without pending it, so two could be approved
  concurrently and the final state become commit-order-dependent. No single-person bypass and
  no sellable exposure follow (`inst-su-commit` is one ACID transaction; sellability
  predicate (4) blocks a retired plan) — which is why this is Cleanup, not the correctness
  finding it was first reported as. PRD AC #112 mirrored.
- **C-4. The approval record's subject discriminator promoted out of free-form JSON.**
  `pricing_approval` carried the kind only inside `materiality` jsonb as "trigger source",
  while the sibling read-model store got a typed, extensible `subject_kind` under D-91.
  Nothing was unimplementable (`inst-as-reject` deliberately delegates dispatch) — schema
  tidiness, not the correctness defect it was first reported as. Column added.
- **C-5. Observability NFR added to the PRD** (`cpt-cf-bss-pricing-nfr-observability`,
  §7.1). The PRD had zero occurrences of observability/monitoring/telemetry while the slices
  declare ~two dozen alarms, several Critical and money-facing
  (`pricing.readmodel.pin_eligibility_overdue`, `pricing.window.changeover_unwarmed`,
  `pricing.audit.chain_gap`…). Note: §7.2's "no silent omissions" list covers domains not
  owned by this PRD, so its silence on observability was not itself the defect — the missing
  §7.1 row was.
- **C-6. Bulk import / mass repricing gain a failure alarm** (`pricing.bulk.run_failed`,
  Warn). The two existing alarms cannot fire on a run that completes promptly with every row
  failed — the exact shape D-111/D-124 create ("a plan whose aggregate pass fails marks all
  of that plan's rows failed") — since `run_stalled` requires absence of progress and
  `conflict_rate_high` keys on ETag conflicts. `pricing_bulk_rows_total{outcome}` existed as
  a metric with no alarm attached.
- **C-7. The D-115 registered enumeration completed** with the plan-change contract content:
  `usage_counter_on_plan_change` (D-113 — flipping `reset → carry` on a graduated target
  changes which band the continued `Q` lands in at zero price-row delta),
  `allowed_change_targets`, `comparability_rank`. Weakened honestly: the rule's blanket
  no-computable-delta clause covers these on a plain reading — this is completeness of the
  concrete list an implementer codes from (S5's reviewability invariant treats it as the
  registered set). S5 + PRD + a dated note in the D-115 entry; no new D number.

## Minor / hardening (latent, not live)

- **N-1. Preview-grant region scoping evaluation defined** (`inst-rb-preview-scope`). Three
  MUSTs said preview is deny-by-default and "region/brand-scoped by claims", and the endpoint
  is catalogued — so the "unauthenticated enumeration" reading was **refuted**. What survived:
  pricing `region` is deliberately decoupled from the authz-region claim (S4 C5), brand "is
  not a price-row field", and `inst-rb-region` constrains mutation only — so a compliant
  implementation could check grant presence + tenant and stop, and a grant issued for one
  market previewed all of them. Fixed: the grant carries an explicit pricing-region set; brand
  has no selector on the base-price surface.
- **N-2. Two strings D-109 left behind.** The retire API row read "Dry-run + confirm
  retirement" and the approval machine's initial state was glossed "opened by a material
  publish" — a retirement is neither. Neither was a claimed propagation site and the
  mechanism is fully specified elsewhere (`inst-re-governed`, `dod-retirement`, the S11 AC,
  PRD). Prose only; both fixed.
- **N-3. The tenant predicate on the migrated-origin read stated.** The one read surface whose
  authz object (`plan`) differs from its path object (a subscription), so row ownership was
  inherited rather than stated. A leak reading was **refuted** (same authority as the read
  model; pre-synthesis GET returns 404 "never a partial or guessed payload"; the endpoint is
  catalogued with `plan × read`). One clause added to `inst-sy-surface`.

## Retracted during verification

- **`pricing_group_membership` cross-tenant enumeration.** `group_value` is FK-like to a
  taxonomy keyed `(tenant_id, value)`; membership has its own stricter `customer_group`
  resource; and `inst-plv-member-preview` already forbids taking the payer identity from a
  client parameter. `payer_tenant_id` is business data in a three-axis tenancy model, not the
  isolation key.
- **Migrated-origin snapshot leak to a cross-tenant service identity.** No document types any
  pricing service identity as tenant-unscoped; see N-3.
- **Lost `CatalogVersion` request with an alarm that cannot fire.** The pending ref is durable
  at publish ("the EventOutbox enqueues `PlanPublished` with a pending version ref", S1 §3.6)
  and the same section speaks of "an expected pending ref" — so
  `pricing.catalogversion.commit_overdue` has a row to fire on; remediation is a registry
  re-request.

Also retracted **as characterisations**: "five slices lost tenant isolation" (→ C-1);
"retirement has no approval mechanism" (→ N-2; the claim that it returns no 202 is simply
false — `inst-rt-return` returns 202); "the approval record cannot distinguish subjects"
(→ C-4); and "two contradictory units can be approved concurrently" as a correctness defect
(→ C-3).

## Unverified — do not action without checking

Surfaced by the non-functional pass, never verified, and plausibly defused by mechanisms not
yet read. Listed so the next pass starts here instead of rediscovering them:

- the **D-114 pin-eligibility frontier** as an unbounded publishing freeze (the decision
  itself records the freeze as an accepted, alarmed cost — expect a downgrade);
- **`overlay_index` size** bounded only by the tenant's live overlay count (D-112's own §7-31c
  perf note suggested an explicit cap would make it a non-question);
- the **20-currency floor vs the 500-rows-per-plan soft cap** arithmetic (the cap is
  explicitly soft and tenant-configurable, and usage rows are phase-invariant — recompute
  before believing it);
- the **24h idempotency TTL** against an approval lifetime with no expiry;
- **`pricing_operator_flag` has no read endpoint**;
- the **GDPR-vs-WORM disposition** covers operator PII only, while payer membership history is
  retained ≥ 7 years.

## Verification & fix record

Fix wave 2026-08-01, on the owner's go; every fix applied against the cited document text.

| Finding | Verdict | Fix | Where it landed |
|---------|---------|-----|-----------------|
| H-1 (anchor/proration N-valued per market) | **CONFIRMED** | **D-123** — the three proration-contract fields uniform per plan-`(currency, region)`; `PRORATION_CONTRACT_MIXED_MARKET`; `billingTiming` exempt; **flagged for veto · joint** | S6 `inst-pi-uniform` (new) + §5 + §6 + `dod-proration-inputs` + AC; PRD `fr-proration-input-contract` + AC #61 |
| M-1 (D-111 justification unsound) | **CONFIRMED** (downgraded — no money-loss scenario) | **D-124(1)** — the aggregate pass runs at entry to `committing`, inside the bulk lock | S12 `inst-mr-validate-scope` + `dod-mass-repricing` + §10 + AC; the D-111 entry pointer |
| M-2 (aggregate input undecidable) | **CONFIRMED** | **D-124(2)** — input = the plan's full post-run row set | same sites + the single-market-selector integration AC |
| M-3 (no pagination contract) | **CONFIRMED** | **D-125** — Foundation cursor contract; SLO per page/chunk | S1 §3.3; S12 ×4; S5 ×2; PRD `fr-price-history-export` + AC #40 |
| C-1 (tenant-rule cross-refs) | **CONFIRMED · Cleanup** (isolation hole refuted) | §6 headers ×10 repointed to Foundation §2.2 + `inst-rb-pep`; S3 §3.1 cite fixed | S2–S5, S7–S12 §6; S3 fixture note |
| C-2 (taxonomy enumerations) | **CONFIRMED · Cleanup** (authz-pair gap refuted) | widened to `{region\|brand\|partner\|orgTier}` | S5 label + endpoint rows; S4 `dod-taxonomy` Touches |
| C-3 (single-pending enumeration) | **CONFIRMED · Cleanup** (correctness impact refuted) | "of any kind … touches the key" | S7 `inst-co-single-pending`; PRD AC #112 |
| C-4 (subject discriminator in jsonb) | **CONFIRMED · Cleanup** | typed `subject_kind` column (D-91 pattern) | S5 §6 `pricing_approval` |
| C-5 (no observability NFR) | **CONFIRMED · Cleanup** | `cpt-cf-bss-pricing-nfr-observability` | PRD §7.1 |
| C-6 (no bulk failure alarm) | **CONFIRMED · Cleanup** | `pricing.bulk.run_failed` (Warn) | S12 §7 + §10 |
| C-7 (D-115 enumeration) | **CONFIRMED · Cleanup** (blanket clause already covers) | plan-change contract content joins the registered set | S5 `inst-mat-registered`; PRD `fr-approval-threshold-policy`; D-115 entry note |
| N-1 (preview region scoping) | **CONFIRMED · Minor** (enumeration reading refuted) | explicit region set on the grant; brand has no selector here | S5 `inst-rb-preview-scope` (new) + role matrix + `dod-rbac` |
| N-2 (two D-109 strings) | **CONFIRMED · Minor** | prose fixed | S11 §5 retire row; S5 approval-machine gloss |
| N-3 (migrated-origin tenant predicate) | **CONFIRMED · Minor** (leak refuted) | tenant binding stated | S11 `inst-sy-surface` |
| Membership cross-tenant enumeration | **RETRACTED** | — | — |
| Migrated-origin cross-tenant leak | **RETRACTED** | — | — |
| Lost-CatalogVersion unfireable alarm | **RETRACTED** | — | — |
