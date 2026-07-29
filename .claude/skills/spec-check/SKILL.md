---
name: spec-check
description: Cross-document invariants over a gear's design set — checks that every decision's claimed propagation surface actually cites it (P1), that every PRD requirement is claimed by exactly one design slice (P2), and that instruction ids and error codes are both declared and referenced (P3). Run on demand against one or more gear `docs/` directories. Trigger on "spec-check", "check the design set", "does the PRD reflect D-NN", "propagation gaps", "unreferenced error codes".
user-invocable: true
allowed-tools: Bash, Read
---

# spec-check

Three deterministic invariants over a gear's `docs/` tree.

**The exit code gates something.** CI's `Spec Invariants` job
(`.github/workflows/docs.yml`) runs `make spec-check` on every PR touching
`**/*.md`, `docs/`, `guidelines/` or `tools/spec-check/`, and a live finding at or
above `--max-severity` (default `medium`) fails it. Pinned known debt never does.
That job still shells out to the Rust crate `tools/spec-check`, which this skill
is a verified port of — rewiring the Makefile to the Python CLI below, and
removing the crate, is a later step.

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

## Tests

```bash
cd .claude/skills/spec-check && python3 -m pytest
```

110 tests, no third-party runtime dependencies. Four of them are the oracles this
port was accepted against, and they are the ones to distrust a change that
reddens them rather than edit:

1. `tests/test_cli.py` — stdout in all three forms, diffed byte-for-byte against
   output frozen from the Rust implementation this was ported from
   (`tests/oracles/`; how it was captured is in `tests/oracles/REGENERATE.md`).
2. `tests/test_propagation.py` — the 24 pinned propagation gaps, reproduced
   exactly against the live corpus.
3. `tests/test_closure.py` — the 51 pinned unreferenced error codes, likewise.
4. `tests/test_backtest.py` — all three invariants against the frozen `10073c36`
   pricing tree under `tools/spec-check/tests/fixtures/`: P1 28, P2 7, P3 55.
