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

**Status: the pipeline is built and the triage thresholds are not yet usable.** On
the live corpora every one of the 116 requirements lands in
`suspicious:multi-region`, which makes three of the six classes unreachable and
would spend a judge call on all 116 — see "Why the thresholds are still open"
below. Steps 1 and 3 are sound and tested; step 2 is not worth running at this
setting.

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

### Why the thresholds are still open

Measured on the live corpora 2026-07-30, at the design's starting values (window
12/6, region threshold 4 distinct terms, `covered:strong` ≥ 8):

| | pricing | ledger |
|---|---|---|
| requirements | 76 | 40 |
| windows in the corpus | 1619 | 1217 |
| terms per requirement, median | 33 (max 161) | 28 (max 141) |
| windows clearing the threshold of 4, per requirement | ~374 | ~239 |
| regions selected per requirement | 3–5, never fewer | 3–5, never fewer |
| triage outcome | 76 × `multi-region` | 40 × `multi-region` |

Classes 4–6 of the ladder (`not-normative`, `weak-coverage`, `covered:strong`) all
require **exactly one** region, so they are unreachable whenever more than one
window clears the threshold — which, at 4 distinct terms out of a median 33, is
always. The thresholds were derived from a run over one-line `**Decision**:`
fields; a requirement paragraph yields an order of magnitude more terms, so the
absolute counts do not transfer. Two further measurements bear on the fix:

- **The document-frequency cutoff barely fires** at this corpus size: median 33
  raw terms → 29 kept, because a term must appear in more than 405 of pricing's
  1619 windows to be dropped.
- **57 % of pricing's term-overlap regions are windows of `PRD.md` itself**
  (125 of 220) — neighbouring requirements sharing vocabulary with side A, not
  design prose. Ledger: 34 % (28 of 83).

A scale-free score (the *fraction* of a requirement's terms a window carries)
separates: the best non-self window covers a median 0.78 of them in pricing and
0.63 in ledger, with a minimum of 0.33. That is a design decision, not a knob, and
it is open.

## Tests

```bash
cd .claude/skills/spec-check && python3 -m pytest
```

183 tests, no third-party runtime dependencies — 110 for the deterministic layer
and 73 for N1. Four of them are the oracles this port was accepted against, and
they are the ones to distrust a change that reddens them rather than edit:

1. `tests/test_cli.py` — stdout in all three forms, diffed byte-for-byte against
   output frozen from the Rust implementation this was ported from
   (`tests/oracles/`; how it was captured is in `tests/oracles/REGENERATE.md`).
2. `tests/test_propagation.py` — the 24 pinned propagation gaps, reproduced
   exactly against the live corpus.
3. `tests/test_closure.py` — the 51 pinned unreferenced error codes, likewise.
4. `tests/test_backtest.py` — all three invariants against the frozen `10073c36`
   pricing tree under `tests/fixtures/`: P1 28, P2 7, P3 55.
