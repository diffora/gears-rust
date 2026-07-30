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

**Status: steps 1 and 3 are built, tested and pinned; step 2 has not been run.**
The live histograms below are frozen in `tests/test_neighbourhoods_cli.py`, and no
requirement has been judged yet — the evaluation the design asks for (the ledger
hypothesis, and a hand-labelled sample of 12) is still owed.

Three steps, two durable artifacts, one judge per requirement:

```bash
# 1. deterministic: neighbourhoods + a triage histogram
python3 .claude/skills/spec-check/scripts/neighbourhoods.py \
  --gear gears/bss/ledger/docs \
  --out .spec-check/neighbourhoods-ledger.json

# 2. judgment: one `spec-check-n1-judge` sub-agent per neighbourhood with
#    "judge": true, its rendered neighbourhood as the entire prompt. Collect the
#    JSON objects into a list and write .spec-check/verdicts-ledger.json.

# 3. deterministic again: validate, enforce the honesty rules, render
python3 .claude/skills/spec-check/scripts/judge_report.py \
  --neighbourhoods .spec-check/neighbourhoods-ledger.json \
  --verdicts .spec-check/verdicts-ledger.json \
  --out docs/spec-check/N1-ledger.md
```

Render step 2's prompt with the code, never by hand:

```bash
python3 - <<'PY'
import json, sys
sys.path.insert(0, ".claude/skills/spec-check/scripts")
from spec_check.semantic import neighbourhood
envelope = json.load(open(".spec-check/neighbourhoods-ledger.json"))
for item in envelope["neighbourhoods"]:
    if neighbourhood.judge_needed(item):
        print("=== {} ===".format(item["id"]))
        print(neighbourhood.render_for_judge(item))
PY
```

`render_for_judge` withholds `selected_by`, `score` and `triage`. That is the
control, not a formatting preference: a judge told a region was id-anchored is
biased toward accepting it, and the premise run was validated blind. Whether
revealing it helps is the one A/B worth running.

**Run it per gear, not per repository.** The artifacts and the report are
gear-scoped, and a judge call per requirement is the cost.

**Where things go, and why it matters:**

| Artifact | Location | Reason |
|---|---|---|
| `neighbourhoods.json`, `verdicts.json` | `.spec-check/` (git-ignored) | Regenerable, and they quote design prose |
| The report | `docs/spec-check/N1-<gear>.md` | **Never inside a gear's `docs/`** — a corpus loads every `*.md` under its root, so a report written there becomes a document the next run parses and term overlap starts matching the previous run's own output. `judge_report.py` refuses such a path outright |

References in the report are plain `path:line` text, not links, because
`make lychee` walks `docs`.

Two rules are enforced by `judge_report.py` rather than by the judge's prompt:

1. **An unbuildable neighbourhood is a finding, never a skip.** `no-prose` and
   `no-region` get their own report rows, with the reason and the line the
   requirement is declared at. Zero neighbourhoods and zero findings must never
   look alike.
2. **A finding that cannot cite two sides is discarded.** A `divergent` verdict
   without `file:line` for the assertion *and* `file:line` for what contradicts
   it, in distinct locations, is downgraded to `consistent` and the downgrade is
   recorded in the report.

Plus one check beyond the design: a citation whose `file:line` falls outside every
fragment of its own neighbourhood makes the verdict `judge-failed`. That is what
turns "the judge has no repository access" from a claim about the harness into
something the pipeline verifies.

### What the thresholds are, and how they were arrived at

A region's score is the **fraction** of the requirement's discriminating terms the
window carries, not a count of them. The design specified absolute counts
(threshold 4 distinct terms, `covered:strong` ≥ 8), derived from a run over
one-line `**Decision**:` fields. Measured 2026-07-30, that does not transfer to
requirements, whose prose yields a median 33 terms in pricing and a maximum of 161:
about 374 of pricing's 1619 windows cleared a threshold of 4 for the median
requirement, top-3 always filled, and **every one of the 116 live requirements came
back with 3–5 regions**. Since three of the six triage classes require *exactly
one* region, they were unreachable, and a requirement's class was in effect a
function of how long its paragraph happened to be.

A fraction is scale-free — a requirement of 8 terms and one of 161 are scored
alike. The two values in use are the knee of the measured distribution, over design
documents only:

| threshold | windows per requirement (pricing / ledger) | requirements with none |
|---|---|---|
| 0.50 | 3.3 / 3.6 | 10 / 12 |
| **0.60** | **1.4 / 0.8** | **24 / 23** |
| 0.70 | 0.6 / 0.3 | 45 / 32 |

`SCORE_THRESHOLD = 0.6` is where 0, 1 and 2 regions all occur naturally; 0.5 still
fills top-3 and 0.7 leaves half of each corpus unmatched. `STRONG_SCORE = 0.75`
leaves roughly 20 of pricing's 76 requirements with a qualifying window.

Term-overlap also **excludes the declaring document wholesale**, not just the
declaration's own window: 57 % of pricing's term-overlap regions (125 of 220) were
windows of `PRD.md` itself — neighbouring requirements sharing vocabulary with the
one being judged — and each spent one of five region slots. N1 asks whether the
*design set* specifies the requirement, and the PRD is side A of that comparison.
Duplication within one document is a real defect and a different check. Id anchors
are unaffected and still reach every document.

One thing the cutoff does *not* do at this corpus size: a term must appear in more
than 405 of pricing's 1619 windows to be dropped, so the document-frequency filter
takes a median 33 raw terms down to 29. It removes ubiquitous words and nothing
else; the scale-free score is what makes the numbers comparable.

### The pinned histograms

| class | pricing | ledger | judged |
|---|---|---|---|
| `unbuildable:no-prose` | 0 | 0 | no |
| `no-region` | 6 | 0 | no |
| `suspicious:multi-region` | 60 | 37 | yes |
| `suspicious:not-normative` | 0 | 0 | yes |
| `suspicious:weak-coverage` | 10 | 3 | yes |
| `covered:strong` | 0 | 0 | no |
| **total** | **76** | **40** | 70 + 40 judge calls |

Hand-checked before being frozen, which is the whole point of a pin:

- The six `no-region` requirements are all pricing **NFRs** — read latency, event
  propagation, multi-currency scale, mass-repricing throughput, availability/DR,
  size limits. Checked negatively: the design slices mention `p95 < 100ms` in ten
  places and every one is a reference to a budget the PRD defines. No slice
  specifies an SLO, so this is the class working, and the finding is real.
- `suspicious:weak-coverage` behaves as designed: `fr-addon-rules` has one anchored
  region carrying **one term of 53** (score 0.019). The id is named; the rule is
  not there.
- Ledger's `fr-idempotency-per-flow` is anchored in four distinct slices scoring
  0.000, 0.571, 0.143 and 0.286 — three of the four name it and say nothing, which
  is exactly what P2 reports as "claimed by five slices" and cannot interpret.
- `covered:strong` and `not-normative` are **honest zeroes**: the first needs
  exactly one region, and a requirement named in one slice usually also has a
  term-overlap region; the second needs non-normative prose, and requirement prose
  in both corpora is overwhelmingly `MUST`-laden. Both stay in the histogram at
  zero so a class that stops occurring is distinguishable from one that never
  existed.

Still 110 judge calls for 116 requirements, so triage is currently filtering little
— the judge's per-region `usefulness`, aggregated by the report, is the channel
that should tighten it, and that requires actually running step 2.

## Tests

```bash
cd .claude/skills/spec-check && python3 -m pytest
```

186 tests, no third-party runtime dependencies — 110 for the deterministic layer
and 76 for N1. Four of them are the oracles this port was accepted against, and
they are the ones to distrust a change that reddens them rather than edit:

1. `tests/test_cli.py` — stdout in all three forms, diffed byte-for-byte against
   output frozen from the Rust implementation this was ported from
   (`tests/oracles/`; how it was captured is in `tests/oracles/REGENERATE.md`).
2. `tests/test_propagation.py` — the 24 pinned propagation gaps, reproduced
   exactly against the live corpus.
3. `tests/test_closure.py` — the 51 pinned unreferenced error codes, likewise.
4. `tests/test_backtest.py` — all three invariants against the frozen `10073c36`
   pricing tree under `tests/fixtures/`: P1 28, P2 7, P3 55.
