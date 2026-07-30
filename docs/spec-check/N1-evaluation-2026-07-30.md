# N1 evaluation step 2 — hand labels against two judge models

Run 2026-07-30 on `gears/bss/pricing/docs` + `gears/bss/ledger/docs` at `d0fa94b0`.
Twelve requirements, labelled by hand before any dispatch, then judged twice on
identical prompts — once by the configured judge (Sonnet at the time) and once with the
model overridden to Opus. Advisory; nothing gates. Read this as an **inter-rater
comparison, not an accuracy score** — the reason is in "What the reference turned out
to be worth".

## The sample and how it was chosen

Within each (gear, triage class) bucket: the first three in corpus order, from the
requirements not already judged in the ledger step-1 run. That gives 6 pricing (3
`multi-region`, 3 `weak-coverage`) and 6 ledger (3 `multi-region`, 3 `weak-coverage`).

This deviates from the plan, which asked for ledger's multi-claim set. That set was
exhausted: 10 of its 16 are `anchored:no-account` and never reach a judge, and the
other 6 were judged in step 1 — labelling them after reading those verdicts would
measure agreement with the judge's framing rather than accuracy. Selecting by triage
class is both available and better targeted, because `suspicious:multi-region` is the
only class where a `divergent` verdict is reachable at all.

All five coverage/agreement classes the schema can express are exercised.

## Results

| # | requirement | hand labels | Sonnet | Opus |
|---|---|---|---|---|
| 1 | pricing fr-custom-frequency | underspecified | underspecified | underspecified |
| 2 | pricing fr-hybrid-completeness | underspecified | underspecified | underspecified |
| 3 | pricing fr-one-time-setup | specified | specified | specified |
| 4 | pricing fr-tier-validation | specified / consistent | specified / consistent | specified / consistent |
| 5 | pricing fr-level-aggregation | specified / consistent | specified / consistent | specified / consistent |
| 6 | pricing fr-pricewindow-coverage | specified / consistent | specified / consistent | specified / consistent |
| 7 | ledger fr-debit-note-charge | **claim-only** | underspecified | underspecified |
| 8 | ledger fr-allocation-precedence | underspecified | underspecified | underspecified |
| 9 | ledger fr-asc606-po-identification | underspecified | underspecified | underspecified |
| 10 | ledger fr-ar-tie-out | specified / **not-applicable** | specified / **consistent** | specified / **divergent** |
| 11 | ledger fr-fx-rate-source-failure | specified / **divergent** | specified / **consistent** | specified / **divergent** |
| 12 | ledger nfr-data-residency | specified / **consistent** | specified / **divergent** | specified / **divergent** |

Pairwise, on both axes: labels↔Sonnet 8/12, labels↔Opus 9/12, Sonnet↔Opus 9/12.

**On `coverage` alone the three sources agree almost completely** — 12/12 between the
two models, 11/12 against the labels. Every remaining disagreement is on `agreement`,
and specifically on whether two accounts contradict each other.

## Divergence detection

Three contradictions were returned across the run. Each was opened in the real
documents and confirmed at both cited locations before being recorded here. **None is a
false positive**, and no source produced a divergence that failed the check.

| source | found | false positives |
|---|---|---|
| hand labels | 1 of 3 | 0 |
| Sonnet | 1 of 3 | 0 |
| **Opus** | **3 of 3** | 0 |

### `fr-ar-tie-out` — found by Opus alone

- `design/01-repository-foundation.md:64` — "Daily `TieOutJob` recomputes every balance
  grain from `journal_line`, **matches exactly**, re-checks no-negative and per-entry
  zero-sum independently, and blocks close on variance/open exceptions."
- `design/07-reconciliation-export.md:380` — "Compare cache vs projection; **evaluate
  tolerance** per X4 (≤ 1 minor unit per 1,000 posted lines rounding-only; statutory
  floors override …)"

`PRD.md:616` requires the balance to "tie (**within tolerance**)", so the Foundation row
contradicts both the PRD and the slice that owns the algorithm. The consequence is
concrete: an implementer working from the Foundation row builds a zero-tolerance close
gate that blocks period close on legitimate residual-cent noise.

### `fr-fx-rate-source-failure` — found by the labels and Opus

- `design/06-fx-multicurrency.md:109` — F4: "Provider-unreachable at S1 | Post
  **blocks** until a rate is available, **except** where tenant policy explicitly
  allows fallback to the last-good rate marked `stale=true`"
- `design/06-fx-multicurrency.md:233` — `inst-rs-local`: "A provider being unreachable
  fails the **sync job** (alarmed), **not a post**; `FX_RATE_UNAVAILABLE` therefore
  means 'no acceptable local rate for the pair', not 'provider TCP timeout'"

The same condition, two incompatible rules. Step 1 of that algorithm carries a revision
marker, so the algorithm was rewritten to be post-path-local and the assumption row was
left behind. F4 names the PRD as its source, and `PRD.md:688` does say the post MUST
block on provider-unreachable — so the algorithm has drifted from both.

### `nfr-data-residency` — found by Sonnet and Opus

- `design/01-repository-foundation.md:73` — "One PostgreSQL cluster per residency cell;
  `period_id` range partitioning **pins residency**", verification "Operational"
- `design/02-audit-immutability-observability.md:137` — G3: "**Deferred post-MVP**:
  tenant data residency is out of MVP scope … there is **no** region-pinning and **no**
  cells … `period_id` partitioning **cannot** pin a tenant inside a shared cluster"

Contradictory on two independent points: whether residency exists in v1 at all, and
whether `period_id` partitioning is the mechanism. The Foundation table presents a
deferred capability as a delivered one, so a reader could onboard a residency-pinned
tenant onto a shared cluster believing the NFR is satisfied.

## What the reference turned out to be worth

The hand labels were wrong three times out of twelve, and the two models caught every
one of those errors:

- **#7**, labelled `claim-only`. Both models read the slice overview as stating
  operative content — corrections are new compensating entries only, posted invoice
  line financials are never mutated — which is two of the declaration's four clauses,
  not a bare mention. `underspecified` is the better call.
- **#10 and #12**, labelled without a divergence that is really there.

One further labelling defect, self-reported: the fragments were dumped truncated while
labelling, so the reasoning written for #9 claimed the SSP-snapshot clause was absent
when the full fragment states it. The verdict (`underspecified`, on the missing
Contract → Catalog → PO-type → billing-model precedence chain) survives; the stated
reason was partly wrong.

**So "agreement with the labels" is not a quality score here.** The reference was
produced by a model with full repository access and it lost to a restricted judge on the
class that matters. The two results this run does support are the pairwise model
comparison and the three eye-verified contradictions — both independent of whether the
labels were right.

## The model decision, and what it is contaminated by

The judge's agent definition moved from `model: sonnet` to `model: opus` on the strength
of the 3-vs-1 divergence detection, at zero false positives for either model. That
decision was taken from the pairwise comparison and from reading the disagreements, not
from either model's score against the labels — deliberately, because the labels are the
weaker instrument.

It remains a choice made on the same sample the measurement ran on, which makes any
headline number optimistic. A confirming run on a fresh sample is owed. What is not
contaminated is the qualitative evidence: on #11, Sonnet's own reasoning asserts that
"Regions 2 and 3 both state the identical rule" about two passages that state mutually
exclusive rules.

The cost consequence is stated plainly in `SKILL.md`: Sonnet was there as a per-token
control, and giving it up leaves dispatch-count control — the ladder and batching — as
the only budget lever. That is the correct place for it, since the dispatch count is
what the bill tracks, but the per-token cost of a run roughly follows the model.

## Resolution limit

Twelve requirements. A one- or two-item difference in these tables is noise and must not
be read as a trend. The 3-vs-1 result is reportable not because 3 > 1 on a sample of 12
but because each of the three is a specific contradiction confirmed by hand in the
documents, and the miss on #11 is visible as a reasoning error rather than a coin flip.

One format deviation, recorded because it costs a batch its retry: Sonnet returned four
newline-delimited JSON objects instead of the requested array on batch-02. Opus returned
a well-formed array on all three batches.

## The search limitation this run exposed

**Corrected 2026-07-30, after investigation.** This section first recorded two
"calibration hypotheses", the second of which was wrong twice over — in its mechanism
and in its facts. Both errors are stated here rather than quietly edited out, because
the wrong version was the more actionable-sounding one and someone would have acted on
it.

**What was claimed:** that the top-3 term-overlap cut "loses a requirement's decisive
region to a neighbour" — `inst-cs-customfreq` states `fr-custom-frequency` in full,
appears in the neighbourhoods of `fr-one-time-setup` (0.833) and
`fr-hybrid-completeness` (0.625), but not its own.

**What is true.** Scoring is per requirement and independent, so no region can be lost
to a neighbour; the same window is simply scored three times against three different
term sets. Measured directly, `design/02-plan-definition.md:199-210` scores **0.414**
for `fr-custom-frequency` — it was cut by the 0.6 threshold, not by the top-3 rule, and
was never a candidate. And `inst-cs-customfreq` does **not** state the requirement in
full: it omits the declaration's first clause, that the catalog must persist the
interval as metadata for `quarterly`, `semiannual` and `customEveryN{...}`, and never
mentions quarterly or semiannual at all. So `underspecified` — the verdict all three
sources gave — was **correct**, and this is not an example of the tool missing a rule
the design set carries.

**The real, milder finding.** A region's score is
`|requirement terms ∩ window terms| / |requirement terms|`, the *recall* of the
declaration's vocabulary. A declaration carrying enumerations, illustrations and
cross-team notes has terms the design legitimately never repeats, so terse normative
prose scores low against a verbose declaration by construction. Here the better
evidence lost to the worse one: the 0.414 window states four of five clauses, while
`DECISIONS.md` D-20 at 0.621 states one — and D-20 was selected. The judge reasoned
from the weaker fragment and still reached the right class.

The same root explains the step-1 finding in `N1-ledger-step1-findings.md`, where
`fr-idempotency-per-flow`'s decisive design-response row scored 0.000: a table cell is
about as terse as design prose gets.

**No change is proposed, and two obvious fixes are backwards.** Scoring a best-matching
sub-span, or making the list item the selection unit, would each *lower* every score,
because recall is monotonic in window size. Lowering the threshold is already measured
bad. The mitigation that exists — id anchors admitted at any score — is what saved the
`fr-idempotency-per-flow` case and is the standing reason not to tighten anchors on
score; it cannot help `fr-custom-frequency`, whose window never names the id.

**One thing worth measuring later:** `_anchor_regions` takes the first id occurrence per
document, usually a `Traces to` header rather than the rule. Preferring the
highest-scoring occurrence looks right — but neither known case would be fixed by it,
since both name the id exactly once, so proposing it now would be the same
evidence-free guess this section already made once.

## A limit of the schema itself

N1's `agreement` axis compares accounts **with each other**. It cannot express a
contradiction between the declaration and its single account. Two requirements in this
sample hit that wall — #8, where the design puts a PRD `MUST` (statutory allocation
precedence) explicitly out of v1 scope, and #12's underlying deferral. Opus named the
gap unprompted: "the text that contradicts the descope is the PRD declaration itself,
which is not a citable region here". Both land as `underspecified`, which is the closest
available verdict and not the right one. A `contradicts-declaration` outcome would need
its own honesty rule, since side A is not a region and the two-sided citation check
would have to change shape.
