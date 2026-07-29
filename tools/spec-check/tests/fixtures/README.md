# `spec-check` backtest fixture — frozen, never hand-edited

`gears/bss/pricing/docs/` under this directory is a **frozen copy of the pricing design set
as of commit `10073c36`** — the tree as it stood *before* the 2026-07-29 review fixes.
`tests/backtest.rs` runs the invariants against it to measure how many of that tree's real,
already-known defects the checker rediscovers.

## Generation

```sh
git archive 10073c36 gears/bss/pricing/docs | tar -x -C tools/spec-check/tests/fixtures
```

Run from the repository root. The contents of `fixtures/gears/` are **byte-identical** to
that command's output — 20 markdown files: `PRD.md`, `DESIGN.md`, `DECISIONS.md`,
`STRIPE-GAP-ANALYSIS.md`, three `ADR/`s and thirteen `design/`s.

Verify at any time with:

```sh
git archive 10073c36 gears/bss/pricing/docs | tar -x -C "$(mktemp -d /tmp/specfx.XXXX)"
# then: diff -r <that dir>/gears tools/spec-check/tests/fixtures/gears
```

## Why it must never be hand-edited

`tests/backtest.rs` pins the score this fixture produces per invariant (`PINNED_P1`,
`PINNED_P2`, `PINNED_P3`, and the derived total), and those assertions fail in **both**
directions — a drop and a rise are equally loud. The design spec
(`docs/superpowers/specs/2026-07-29-bss-spec-ir-verification-design.md`) gates promotion of
this whole mechanism on that number.

The only reason those pins cannot flake is that the input cannot change. So:

- **Never edit a file under `gears/` here** — not to fix a typo, not to silence a finding,
  not to make a checker change "work". These documents are a historical measurement, not a
  design set anyone maintains. The live design set is at `gears/bss/pricing/docs/` in the
  repository root; that is the one to fix.
- **Never regenerate it at a different commit.** Re-pointing the fixture at a newer tree
  silently changes what every pinned count means while leaving the numbers looking stable.
  A different baseline commit is a new fixture directory and a new, separately-justified set
  of pins.
- **A pinned count may only move when the checker changed**, with the new number verified by
  hand against this corpus and the reason recorded — see the doc comment on `PINNED_P1` in
  `tests/backtest.rs`, which names each fix that has moved a count so far.

If a change to `invariants::{propagation,fr_coverage,closure}` makes a pinned count move,
that is a real claim about the checker's effectiveness. Verify it, then update the constant
deliberately, in the same commit, with the reasoning in the commit message. Never edit a pin
to re-baseline a failing run back to green, and never edit this fixture to protect a pin.
