---
name: spec-check
description: Cross-document invariants over a gear's design set — checks that every decision's claimed propagation surface actually cites it (P1), that every PRD requirement is claimed by exactly one design slice (P2), and that instruction ids and error codes are both declared and referenced (P3). Run on demand against one or more gear `docs/` directories. Trigger on "spec-check", "check the design set", "does the PRD reflect D-NN", "propagation gaps", "unreferenced error codes".
user-invocable: true
allowed-tools: Bash, Read
---

# spec-check

Three deterministic invariants over a gear's `docs/` tree.

**Advisory — nothing runs this for you, and nothing consumes its exit code.** It
used to: CI's `Spec Invariants` job in `.github/workflows/docs.yml` ran
`make spec-check` on every PR touching `**/*.md`, `docs/` or `guidelines/`, and a
live finding at or above `--max-severity` failed it. That job, the Makefile target
and the Rust crate this is a verified port of (`tools/spec-check`) were all removed
on 2026-07-29, deliberately: the tool is meant to be run on demand against the
documents you are actually working on, one feature at a time.

What that costs, stated plainly rather than glossed: the design set is now
validated by nothing automatically, and the pinned baselines below no longer fail
in both directions on every docs PR — a finding that stops reproducing will just
quietly stop reproducing until someone runs this. The exit code still means what it
always meant; there is simply no longer a gate reading it.

## How to invoke

Run from the repository root, passing every gear whose documents may be involved:

```bash
python3 .claude/skills/spec-check/scripts/check.py \
  --gear gears/bss/pricing/docs \
  --gear gears/bss/rating/docs \
  --gear gears/bss/subscriptions/docs
```

Repo-relative `--gear` paths, from the repository root, on purpose: two findings
(`P1/decision-register-unparsed`, `P2/traceability-convention-unknown`) echo the
corpus root verbatim, so absolute paths give equally correct but non-reproducible
output — and the frozen oracles in `tests/oracles/` only diff clean against this
invocation.

**Pass every related gear, not just the one under review.** Two checks are
cross-gear: P1 resolves a `SEAMS <id>` propagation target against whichever
loaded gear actually defines that seam row, and P3 resolves instruction-id
declarations across the whole loaded set. Run alone, a gear's honest cross-gear
citations become findings against it. Measured on the live corpus as it stands,
pricing alone reports 4 `P1/seam-undefined` — D-44 `SEAMS M10`, D-46 `SEAMS RG3`,
D-60 `SEAMS M12` and D-65 `SEAMS SUB-P7`, every one of them a row that does
exist: in rating's `SEAMS.md` for the first three, subscriptions' for the fourth.
Rating alone reports 7 `P3/inst-dangling`, subscriptions alone 1. All 12
disappear when the three gears are loaded together. Honest, but useless.

`P1/propagation-target-not-loaded` is the same class for a *file* target no loaded
gear provides; this repository's corpora currently produce none of it.

Flags:

| Flag | Effect |
|---|---|
| `--format json` | Machine-readable envelope instead of the text report |
| `--show-known-debt` | Also print the pinned D-69 findings the default output only counts. Never changes the exit code |
| `--max-severity low\|medium\|high` | Lowest severity that fails the run (default `medium`). Pinned known debt is exempt at every level |

Exit codes: `0` clean, `1` the gate tripped **or** a `--gear` directory does not
exist (that one also prints `Error: …` on stderr — a docs tree that will not load
is never reported as a clean run), `2` usage error.

## Reading the output

Findings are `[Severity] file:line — message (invariant)`, or `[Severity] file —
message (invariant)` when the finding is about a whole document rather than one
line. The invariant tag is the stable part; grep for it.

Two lines at the end matter as much as the findings:

- `N finding(s)` — the live set.
- `N known-debt finding(s) suppressed, tracked as D-69 — pass --show-known-debt
  to see them` — pinned, accepted debt (24 propagation gaps + 51 unreferenced
  error codes = 75), withheld by default. The three-gear invocation above
  currently prints `15` and `75`.

Several invariants report **their own coverage** rather than staying silent when
they cannot read something. These are not defects in the documents; they say what
went unchecked:

- `P1/decision-register-unparsed` — a `DECISIONS.md` exists but yielded no
  `#### <id>` entries. Rating's `| T-D-NN |` table legitimately produces this.
- `P1/propagation-uninterpretable` — a citation naming nothing the resolver knows.
- `P2/traceability-convention-unknown` — the gear uses a traceability convention
  P2 does not parse, so per-id claims are not reported for it at all.

## What it cannot tell you

P1 reports that `D-02` claims propagation into `PRD.md` while `PRD.md` never
mentions `D-02`. It **cannot** say whether the rule is present in prose and only
the citation is missing, or whether the rule was never written. Those need
opposite fixes. That judgment is the semantic layer's job, designed in
`docs/superpowers/specs/2026-07-29-spec-check-semantic-layer-design.md` and not
yet built.

Nor can it see a *whole-field* omission: a decision that never names a target at
all is invisible to P1 by construction, because P1 checks the targets a decision
names. D-53 is the canonical example.

## N1 — is the requirement actually specified, and does every account agree?

Everything above is the deterministic layer: it counts claims. N1 reads prose. It
answers the two questions P2 cannot — *is there valid prose specifying this
requirement*, and *do all the statements about it agree* — for the **requirement**
kind (`fr` and `nfr`). Designed in
`docs/superpowers/specs/2026-07-30-spec-check-n1-requirements-design.md`.

**Advisory, like everything else here. Nothing gates. The report is the output.**

**Status:** steps 1, 2 and 4 below are built, tested and pinned. Step 3 — the judge
dispatches — has never been run, so no requirement has a verdict yet and the
evaluation the design asks for is still owed.

### The runbook

Four commands, two of them deterministic, one dispatch loop. Run from the
repository root.

```bash
# 1. build neighbourhoods + print the triage histogram
python3 .claude/skills/spec-check/scripts/neighbourhoods.py \
  --gear gears/bss/ledger/docs \
  --out .spec-check/neighbourhoods-ledger.json

# 2. group the judged ones into batch prompts, one file per dispatch
python3 .claude/skills/spec-check/scripts/judge_batches.py \
  --neighbourhoods .spec-check/neighbourhoods-ledger.json \
  --out-dir .spec-check/batches-ledger

# 3. for each .spec-check/batches-ledger/batch-NN.md: dispatch the
#    `spec-check-n1-judge` sub-agent with that file's contents as the ENTIRE
#    prompt. Collect every returned object into one JSON list and write it to
#    .spec-check/verdicts-ledger.json. Do them one batch at a time, so a bad
#    first result stops the run instead of paying for all of them.

# 4. validate, enforce the honesty rules, render
python3 .claude/skills/spec-check/scripts/judge_report.py \
  --neighbourhoods .spec-check/neighbourhoods-ledger.json \
  --verdicts .spec-check/verdicts-ledger.json \
  --batches .spec-check/batches-ledger/manifest.json \
  --out docs/spec-check/N1-ledger.md
```

To run a hand-picked sample instead of a whole gear, put the ids in a file, one per
line, and pass `--only-id-file`. Do **not** build `--only-id` flags in a shell loop:
zsh does not word-split an unquoted variable, so the loop passes one long string as
a `--gear` path and the run dies confusingly. A requested id no gear declares is an
error, never a silently smaller sample.

```bash
python3 .claude/skills/spec-check/scripts/check.py --gear gears/bss/ledger/docs --format json \
  | python3 -c "import json,sys,re; print('\n'.join(sorted(re.search(r'(cpt-cf-[a-z0-9-]+)', f['message']).group(1) for f in json.load(sys.stdin)['findings'] if f['invariant']=='P2/fr-multiply-claimed')))" \
  > .spec-check/step1-ids.txt
python3 .claude/skills/spec-check/scripts/neighbourhoods.py --gear gears/bss/ledger/docs \
  --only-id-file .spec-check/step1-ids.txt --out .spec-check/neighbourhoods-ledger-step1.json
```

### What a dispatch costs, and how the cost is controlled

Every dispatch pays for its own context — system prompt, tool schemas, the agent's
instructions — before it reads a single fragment. That fixed cost dominates a
neighbourhood's ~1.3–2.4k tokens, so the number of *dispatches* is the bill, not the
payload. Three controls, in order of effect:

| Control | Effect, measured |
|---|---|
| The ladder judges only what needs judging | 116 → **69** judged (52 pricing, 17 ledger) |
| `judge_batches.py`, 4 per dispatch | 69 → **22 dispatches** (14 pricing, 8 ledger) |
| The judge runs on **Sonnet**, not the inherited model | per-token, not per-call |

The ledger evaluation sample the design asks for — the 16 requirements P2 reports as
multiply claimed — comes to **6 judged, 5 dispatches**, because 10 of the 16 are
answered deterministically.

Batching is a **deliberate deviation from the design**, which requires one
neighbourhood per dispatch so verdicts stay independent. It is bounded
mechanically: `judge_batches.py` never puts two neighbourhoods that quote
overlapping lines of any document into one dispatch, so a conclusion about one
paragraph cannot be carried into a verdict about another requirement quoting that
same paragraph. `judge_report.py --batches` prints the deviation into the report —
one that the reader cannot see is one nobody can weigh. For a measurement that must
be beyond argument, pass `--size 1` and pay the full price.

Note what *sounds* stricter and is useless: batching on shared *files* rather than
overlapping spans. Every requirement of a gear declares itself in the same `PRD.md`,
so every pair conflicts and every batch holds one member — measured, ledger's 17
judged neighbourhoods produced 17 batches.

A malformed response gets **one retry**, then the neighbourhood is left out of
`verdicts.json` and `judge_report.py` records it as `judge-failed`. With a batch,
the retry costs the whole batch, which is the argument against large `--size`.

### The ladder

Every requirement lands in exactly one class; the first condition that holds decides.
A **region** is a window that carries at least `SCORE_THRESHOLD` of the
requirement's discriminating terms; an **account** is a region that clears that bar.
An id anchor is admitted as a region whatever it scores, because naming the id is
precise evidence of intent — but a citation is not an account, and only accounts can
contradict each other.

| # | Class | Condition | Judge |
|---|---|---|---|
| 1 | `unbuildable:no-prose` | the declaration has no prose block | no — a PRD defect |
| 2 | `no-region` | prose exists, nothing matched at all | no — reported with its reason |
| 3 | `anchored:no-account` | the id is named, no region is an account | no — reported with its citations |
| 4 | `suspicious:multi-region` | two or more accounts → divergence possible | **yes** |
| 5 | `suspicious:not-normative` | one account, prose carries no MUST/SHALL/SHOULD | **yes** |
| 6 | `suspicious:weak-coverage` | one account, not both anchored and ≥ `STRONG_SCORE` | **yes** |
| 7 | `covered:strong` | one account, id-anchored, ≥ `STRONG_SCORE`, normative | no — the reason is recorded |

`anchored:no-account` is not in the design. The corpus demanded it: 41 of 116 live
requirements are named in the design set while no region carries enough of their
vocabulary to be an account of them. It must not be reported as `claim-only` —
that is a judgment about the documents, and this is a fact about the search.

### Thresholds, and how they were arrived at

A region's score is the **fraction** of the requirement's discriminating terms the
window carries, not a count. The design specified absolute counts (region ≥ 4
distinct terms, `covered:strong` ≥ 8), derived from a run over one-line
`**Decision**:` fields. Measured 2026-07-30, that does not transfer: requirement
prose yields a median 33 terms in pricing and up to 161, so ~374 of pricing's 1619
windows cleared a threshold of 4 for the median requirement, top-3 always filled,
and **every one of the 116 requirements came back with 3–5 regions** — three of the
classes above require exactly one account, so they were unreachable, and a
requirement's class was in effect a function of how long its paragraph was.

A fraction is scale-free: a requirement of 8 terms and one of 161 are scored alike.
The values in use are the knee of the measured distribution over design documents:

| threshold | windows per requirement (pricing / ledger) | requirements with none |
|---|---|---|
| 0.50 | 3.3 / 3.6 | 10 / 12 |
| **0.60** | **1.4 / 0.8** | **24 / 23** |
| 0.70 | 0.6 / 0.3 | 45 / 32 |

`SCORE_THRESHOLD = 0.6` is where 0, 1 and 2 accounts all occur naturally; 0.5 still
fills top-3 and 0.7 leaves half of each corpus unmatched. `STRONG_SCORE = 0.75`.

Term overlap **excludes the declaring document wholesale**, not just the
declaration's own window: 57 % of pricing's term-overlap regions (125 of 220) were
windows of `PRD.md` itself — neighbouring requirements sharing vocabulary with the
one being judged — each spending one of five region slots. N1 asks whether the
*design set* specifies the requirement, and the PRD is side A. Duplication within one
document is a real defect and a different check. Id anchors reach every document.

One thing the document-frequency cutoff does *not* do at this corpus size: a term
must appear in more than 405 of pricing's 1619 windows to be dropped, so it takes a
median 33 raw terms to 29. It removes ubiquitous words and nothing else.

### The pinned histograms

Frozen in `tests/test_neighbourhoods_cli.py`, together with the judge-call count —
that number is what the ladder exists to control, so a change that quietly doubles
it must read as a diff.

| class | pricing | ledger |
|---|---|---|
| `unbuildable:no-prose` | 0 | 0 |
| `no-region` | 6 | 0 |
| `anchored:no-account` | 18 | 23 |
| `suspicious:multi-region` | 34 | 9 |
| `suspicious:not-normative` | 0 | 0 |
| `suspicious:weak-coverage` | 18 | 8 |
| `covered:strong` | 0 | 0 |
| **total / judged** | **76 / 52** | **40 / 17** |

Hand-checked before being frozen, which is the whole point of a pin. Two findings
came out of that check, before any judge ran:

- **All six `no-region` requirements are pricing NFRs** — read latency, event
  propagation, multi-currency scale, mass-repricing throughput, availability/DR, size
  limits. Checked negatively: the slices mention `p95 < 100ms` in ten places and
  every one is a reference to a budget the PRD defines. No slice specifies an SLO.
- **Ten of the sixteen requirements P2 reports as claimed by 2–5 ledger slices are
  `anchored:no-account`**: the claims are real and not one of those locations states
  the rule. That is most of the ledger hypothesis answered deterministically.

`covered:strong` and `not-normative` are honest zeroes, kept in the histogram at
zero so a class that stops occurring is distinguishable from one that never existed.

### Where things go, and why it matters

| Artifact | Location | Reason |
|---|---|---|
| `neighbourhoods.json`, `verdicts.json`, batch prompts | `.spec-check/` (git-ignored) | Regenerable, and they quote design prose |
| The report | `docs/spec-check/N1-<gear>.md` | **Never inside a gear's `docs/`** — a corpus loads every `*.md` under its root, so a report written there becomes a document the next run parses and term overlap starts matching the previous run's own output. `judge_report.py` refuses such a path |

References in the report are plain `path:line` text, not links, because
`make lychee` walks `docs`.

### What the pipeline enforces rather than asks for

- **`selected_by`, `score` and the triage class are withheld from the judge**, by
  `render_for_judge` and therefore also from every batch prompt. A judge told a
  region was id-anchored is biased toward accepting it, and the premise run was
  validated blind. Whether revealing it helps is the one A/B worth running.
- **An unbuildable neighbourhood is a finding, never a skip.** `no-prose`,
  `no-region` and `anchored:no-account` get report rows with their reason and the
  line the requirement is declared at.
- **A finding that cannot cite two sides is discarded.** A `divergent` verdict
  without `file:line` for the assertion *and* for what contradicts it, in distinct
  locations, is downgraded to `consistent` and the downgrade is printed.
- **A citation outside every fragment of its own neighbourhood is `judge-failed`.**
  Beyond the design, and it is what turns "the judge has no repository access" from a
  claim about the harness into something the pipeline verifies.

### What N1 still cannot tell you

- **Decisions (N2) and error codes (N3)** are designed
  (`docs/superpowers/specs/2026-07-29-spec-check-semantic-layer-design.md`) and
  unbuilt. N1 went first because the requirement declaration exists in 25 of 27 PRDs
  while a `DECISIONS.md` register exists in 3 of 28.
- **A rule stated in vocabulary the PRD does not share** lands in `no-region` or
  `anchored:no-account`, which say exactly that and no more. Neither is evidence of
  absence.
- **Whether the judge is any good.** Nothing has been judged. The `usefulness`
  column the report aggregates is the tuning channel, and it is empty until step 3
  runs.

## Tests

```bash
cd .claude/skills/spec-check && python3 -m pytest
```

204 tests, no third-party runtime dependencies — 110 for the deterministic layer
and 94 for N1. Four of them are the oracles this port was accepted against, and
they are the ones to distrust a change that reddens them rather than edit:

1. `tests/test_cli.py` — stdout in all three forms, diffed byte-for-byte against
   output frozen from the Rust implementation this was ported from
   (`tests/oracles/`; how it was captured is in `tests/oracles/REGENERATE.md`).
2. `tests/test_propagation.py` — the 24 pinned propagation gaps, reproduced
   exactly against the live corpus.
3. `tests/test_closure.py` — the 51 pinned unreferenced error codes, likewise.
4. `tests/test_backtest.py` — all three invariants against the frozen `10073c36`
   pricing tree under `tests/fixtures/`: P1 28, P2 7, P3 55.
