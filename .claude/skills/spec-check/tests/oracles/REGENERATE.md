# Frozen oracles — captured, never hand-edited

The three files beside this one are **byte-exact stdout of the checker** against
the live `gears/bss/{pricing,rating,subscriptions}/docs` trees. They have had two
lives:

1. **2026-07-29 — port verification.** The originals were captured from the Rust
   binary `tools/spec-check` as it shipped at commit `06c46d6d`, and the Python
   port was accepted by diffing byte-for-byte against them. That is why the port
   is *verified*, not rewritten: the 1795 lines of production Rust encoded
   behaviour that cost nine commits, five of them fixes, and one Critical review
   finding. The equivalence proof happened then and is banked; recovering the
   binary that produced those originals means checking out `06c46d6d`.

2. **2026-07-31 — regression pins of the Python implementation.** The live
   documents then legitimately moved (the 2026-07-30 slice-review fix round:
   D-79…D-86, the veto confirmations, and the spec-check finding fixes), which the
   Rust-era oracles could never track — the binary is gone. The current files were
   re-captured from `scripts/check.py` against the post-fix trees, together with
   one resolver extension (explicit `../../<gear>/docs/<file>.md` propagation
   targets, `targets.py`). From here on the oracles pin **this implementation's
   own output**, not Rust equivalence.

The discipline is unchanged either way: a finding appearing or disappearing here
is a real claim about the design set, and re-freezing must be a deliberate,
separately-justified commit — never a way to make a failing change look green.

## How they were produced (2026-07-31 capture)

Run from the repository root:

```sh
O=.claude/skills/spec-check/tests/oracles
python3 .claude/skills/spec-check/scripts/check.py \
  --gear gears/bss/pricing/docs \
  --gear gears/bss/rating/docs \
  --gear gears/bss/subscriptions/docs > $O/live-text.txt
python3 .claude/skills/spec-check/scripts/check.py ... --format json > $O/live-json.json
python3 .claude/skills/spec-check/scripts/check.py ... --show-known-debt > $O/live-show-known-debt.txt
```

The `--gear` paths are **repo-relative on purpose**. Two findings
(`P1/decision-register-unparsed`, `P2/traceability-convention-unknown`) echo the
corpus root verbatim, so absolute paths would produce different — equally
correct, but non-reproducible — output.

## When these may change

Only when the *live documents* change (a docs round moving the D-69-tracked debt,
a new review wave) or when the checker deliberately learns a new citation form —
and in either case the re-freeze commit must say which findings moved and why,
with the moved pinned-list members hand-checked (see the notes beside
`PINNED_PROPAGATION_GAPS_2026_07_29` and `PINNED_UNREFERENCED_CODES_2026_07_29`).
