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

## Calibration hypotheses, recorded and not applied

Applying either of these to the sample the measurement ran on would fit the tool to its
own test set. Both need a fresh corpus.

1. **A low score is not low relevance** for the compressed vocabulary of a
   design-response table. See `N1-ledger-step1-findings.md`: the decisive region for
   `fr-idempotency-per-flow` scored 0.000 and was present only because it names the id.
   The id-anchor rule must not be tightened on score.
2. **The top-3 term-overlap cut can lose a requirement's decisive region to a
   neighbour.** `inst-cs-customfreq` states `fr-custom-frequency` in full and appears in
   the neighbourhoods of `fr-one-time-setup` (0.833) and `fr-hybrid-completeness`
   (0.625) — but not in its own, which is why #1 is labelled `underspecified` by all
   three sources when the design set does in fact carry the rule.

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
