# Frozen oracles — captured from the Rust binary, never hand-edited

The three files beside this one are **byte-exact stdout of `tools/spec-check`**
as it shipped at commit `06c46d6d`, captured on 2026-07-29 against the live
`gears/bss/{pricing,rating,subscriptions}/docs` trees.

They exist because the Python port is *verified*, not rewritten: the 1795 lines
of production Rust encode behaviour that cost nine commits, five of them fixes,
and one Critical review finding. Re-deriving that from prose reproduces the same
mistakes unnoticed. The diff against these files is the test.

## How they were produced

Run from the repository root, before `tools/spec-check` was removed:

```sh
O=.claude/skills/spec-check/tests/oracles
cargo run -q -p spec-check -- \
  --gear gears/bss/pricing/docs \
  --gear gears/bss/rating/docs \
  --gear gears/bss/subscriptions/docs > $O/live-text.txt
cargo run -q -p spec-check -- ... --format json > $O/live-json.json
cargo run -q -p spec-check -- ... --show-known-debt > $O/live-show-known-debt.txt
```

The `--gear` paths are **repo-relative on purpose**. Two findings
(`P1/decision-register-unparsed`, `P2/traceability-convention-unknown`) echo the
corpus root verbatim, so absolute paths would produce different — equally
correct, but non-reproducible — output.

## When these may change

Only when the *live documents* change, which is what the D-69 docs round will
do. A finding appearing or disappearing here is a real claim about the design
set, and re-freezing must be a deliberate, separately-justified commit — never a
way to make a failing port look green. After `tools/spec-check` is removed the
binary that produced these no longer exists; recovering it means checking out
`06c46d6d`.
