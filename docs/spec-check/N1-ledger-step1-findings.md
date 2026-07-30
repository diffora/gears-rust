# N1 evaluation step 1 — the ledger hypothesis

Reading of the generated report `docs/spec-check/N1-ledger-step1.md`. Run on
2026-07-30 against `gears/bss/ledger/docs` at `d0fa94b0`. Advisory; nothing gates.

## The question

P2 reports 16 ledger requirements as `fr-multiply-claimed` — each named by two to
five design slices. P2 counts claims and cannot read prose, so it cannot say what
those claims mean. Three outcomes were possible, and they demand opposite fixes:

1. **mostly one exposition** → `**Traces to**:` means "relates to" in ledger, P2's
   "exactly one slice" rule measures the wrong thing there, and the fix belongs to
   the checker rather than to the documents;
2. **mostly two or more, agreeing** → duplicated specification: not a defect today,
   a standing source of future divergence;
3. **any genuine disagreement** → a real document defect.

P2 selected the sample. The judge decided what it means.

## The sample is 16; only 6 were judged

**Ten of the sixteen are `anchored:no-account`** — the design set names the id, and
no location naming it carries enough of the requirement's vocabulary to be an
account of it. That is answered deterministically, before any judge runs, and it is
most of the hypothesis: the claims are real and none of those locations states the
rule. The six judged requirements are **not** the whole sample, and no conclusion
below is a statement about all sixteen.

`anchored:no-account` is a fact about the search, not a verdict that the requirement
is unspecified. It says term overlap found no account; it does not say none exists.

Zero requirements in the sample landed in `suspicious:multi-region` — that is, **no
requirement here has two accounts by the deterministic test**. Outcome 2 was
excluded before the first dispatch.

## The answer

- **Regions marked `specifies`, per judged requirement: 3, 2, 2, 2, 2, 1 — but
  exactly one `decisive` account each, in five of the six.** The only requirement
  with two decisive accounts is `fr-reversal-canonical-pattern`, where the judge
  read them as the Foundation mechanism and the S1 domain flow built on it, not as
  competing statements. Every other `specifies` mark is a summary of the decisive
  account, or its instantiation for one flow.
- **Outcome 1 held.** In ledger, `**Traces to**:` means "this slice relates to that
  requirement", not "this slice specifies it". P2's exactly-one-claiming-slice rule
  is therefore measuring the wrong thing for this gear, and the 16 findings are not
  16 document defects.
- **The fix belongs to the checker, for ledger.** This says nothing about the other
  23 gears: the convention elsewhere may be exactly what P2 assumes, and nothing
  here was measured outside ledger.

One `divergent` was returned. It is real (checked by eye, below) but it is not
evidence for outcome 3 as posed: it is a disagreement inside a single document about
implementation status, not two claiming slices specifying the requirement
differently.

## The `divergent`, checked by eye

`fr-exception-suspense-handling`, both sides in `design/07-reconciliation-export.md`:

- `design/07-reconciliation-export.md:82` — "**`EXPORT_PAYLOAD_CONFLICT` (409)**
  problem code and the **`EXPORT_FAILURE`** exception-queue type — not emitted in v1."
- `design/07-reconciliation-export.md:322` — "**Export failure:** retry-safe, alert,
  queue — no silent drop."

Both quotes reproduce verbatim in the document. **Not a false positive**, and the
finding survives context the judge never saw:

- Line 322 sits in an unmarked list whose other six bullets describe live v1
  behaviour (`MISSED_POSTING`, `STUCK_REFUND_CLEARING` and the mapping-gap and
  reconciliation-mismatch items are all in the half the same note confirms is in v1).
  One bullet of seven is deferred and carries no marker.
- The blanket disclaimer at `design/07-reconciliation-export.md:87` — "Everything
  below describing the export surface is design-forward, not v1" — scopes by topic,
  and the ExceptionQueue enumeration is not the export surface. It belongs to the
  reconciliation half the same note says **is** in v1, so the disclaimer's own wording
  routes a reader toward treating the bullet as live.

Severity is low and the shape is specific: a missing v1-status annotation on one
bullet, not two incompatible statements of a business rule. `design/07-reconciliation-export.md:331`
already annotates the export flow section this way; the ExceptionQueue bullet was
missed.

## The `underspecified`, checked by eye

`fr-manual-adjustment-governance`. `PRD.md:732-736` requires governed adjustments to
enforce "segregation of duties …, amount/entity thresholds with dual-control, and
mandatory reason code + actor + **before/after audit**". Step `inst-gov-sod` at
`design/05-adjustments-notes-refunds.md:415-426` tracks the first three clauses almost
word for word and stops before the fourth.

Checked against the whole gear, not just the neighbourhood: the finding holds, and for
a stronger reason than the judge could see. `design/02-audit-immutability-observability.md:501`
enumerates what the secured audit store must hold as "manual-adjustment reason/actor,
controlled-metadata before/after" — before/after is specified, but for G4 metadata
changes, while manual adjustments get reason and actor only.

**The counter-argument, recorded because it is strong:** corrections in this design are
always new compensating entries and nothing is mutated in place, so a before/after
image of a changed record may have no referent for governed adjustments. The PRD clause
may be boilerplate that does not apply. Real omission, arguable severity — a documented
decision either way would close it.

## What the run measured about the pipeline itself

Nothing had ever been judged before this run, so the first dispatch was also the first
evidence about the machinery. Three findings, all about the search rather than the
documents:

**1. The scorer's account and the judge's account can be disjoint.** For
`fr-idempotency-per-flow`:

| region | selected_by | score | judge |
|---|---|---|---|
| design/01-repository-foundation.md:49-60 | id-anchor | 0.000 | specifies, decisive |
| design/01a-invoice-posting.md:55-66 | id-anchor | 0.571 | specifies, useful |
| design/03-payments-allocation.md:61-72 | id-anchor | 0.143 | mentions, noise |
| design/04-asc606-recognition.md:55-66 | id-anchor | 0.286 | mentions, useful |
| DESIGN.md:43-54 | term-overlap | 0.714 | mentions, **noise** |

The only window clearing `SCORE_THRESHOLD = 0.6` is a prose overview listing
"idempotency contract" among the Foundation's responsibilities; the judge called it
noise. The window stating the rule — `idempotency_dedup` PK `(tenant_id, flow,
business_id)` — scored **0.000** and entered the neighbourhood only because it names
the id. The ladder's rule that an id anchor is admitted whatever it scores is what
saved this measurement; without it the decisive region would not have been in the
prompt. This argues against tightening the anchor rule and against reading a low
score as low relevance for table-shaped design responses, whose vocabulary is
compressed.

**2. `judge_report.py` reported `anchored:no-account` as `covered:strong`.** The
not-judged branch handled `no-prose` and `no-region` and sent everything else to the
`covered:strong` reason text — so 10 of the 16 requirements were described as having
"a single id-anchored region scoring at or above the strong threshold", the exact
reverse of the truth. The seventh class was added to the ladder after the report
renderer was written and the renderer was never updated. Fixed; the class now renders
its own section, and `test_anchored_no_account_is_not_reported_as_covered` fails
against the old code.

**3. `proposed_fix` was required unconditionally.** The agent contract calls the key
optional when coverage is `specified` and agreement `consistent`, and `normalise`
already implements exactly that rule — but a presence check in `_check_schema` fired
first, so a *missing* key failed where an *empty* one passed. It cost
`fr-idempotency-per-flow` — the requirement this run existed to answer — a
`judge-failed` row for a verdict its own contract calls valid. Fixed by removing the
duplicated check and leaving the rule in one place.

**4. The single-requirement batch template did not ask for the prefixed id.** The
report matches verdicts to neighbourhoods on the neighbourhood id
(`requirement/<id>`), but only the multi-requirement template told the judge to copy
the id from its `=== … ===` header; the prompt body's `Requirement:` line carries the
bare form. All six verdicts needed mechanical renormalisation — a substitution that
touches no judgment — before the report would render. Fixed: both templates now name
the prefix and say why, pinned by
`test_both_prompt_templates_demand_the_prefixed_id`.

None of the four was caught by the 205 tests standing before this run, because all
four live where the pipeline meets a real judge, and no judge had ever run. That is
the argument for the second evaluation rather than against the first.
