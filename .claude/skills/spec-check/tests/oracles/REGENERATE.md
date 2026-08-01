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

3. **2026-07-31, second capture — after the day's three pricing review fix
   rounds** (D-87…D-98, D-99…D-112, D-113…D-122). Four pinned-debt members were
   paid down by those rounds and left the pins with hand-checked notes beside
   each list (`D-25 -> PRD.md`, `D-40 -> design/10`, `METER_AMBIGUOUS`,
   `TAXONOMY_VALUE_IN_USE`), pricing-alone seam-undefined moved 6 → 8 (D-93/D-94
   citations, both resolving under auto-context), and the suppressed count moved
   73 → 69. Live findings are unchanged at 7 (the cross-gear coverage
   statements). No checker code changed in this capture — documents only.

4. **2026-07-31, third capture — after two PR-review checker fixes** (the only
   capture so far justified by *checker* behavior rather than document
   movement, both inherited verbatim from the Rust source this port was
   verified against). (i) The citation regex no longer matches a decision id
   as a suffix of a hyphenated sibling-gear id (`\bD-14\b` matched inside
   `T-D-14`, `\bD-01\b` inside `SUB-D-01`) — this exposed one genuinely
   uncited claim, `D-14 -> PRD.md`, which was fixed by citing D-14 at
   `fr-audit-completeness` (live) and moved the frozen-fixture backtest pin
   P1 28 → 29 (the fixture PRD's only `D-14` token is `T-D-14`, hand-checked).
   (ii) The `code-convention-divergent` check now judges a blockless design
   slice against the cross-corpus code-declaration union (mirroring
   `DeclaredInstructions`), so rating `design/04-overlays-precedence.md` —
   whose one prose code is block-declared in pricing — stopped drawing a false
   positive; rating `design/15` still fires (its `RATED` token is declared
   nowhere). Live findings 7 → 6; suppressed unchanged at 69. Also
   `known_debt_suppressed` now reads 0 under `--show-known-debt` (nothing is
   withheld from that envelope).

5. **2026-08-01, fifth capture — after the d-wave billing-domain review fix
   round** (D-123…D-125 + the cleanup tier). Document movement only, no checker
   change. One pinned-debt member paid down and removed with a hand-checked
   note beside the pinned list: `REGION_SCOPE_DENIED` / design/05 — the new
   `inst-rb-preview-scope` rule (N-1, the preview grant's explicit
   pricing-region set) names the code in its rule body, so the 403 finally has
   the rule that fires it. Suppressed 69 → 68; live findings unchanged at 6
   (the cross-gear coverage statements). The neighbourhood pins moved in the
   same commit (total 76 → 77 for the new `nfr-observability`; three triage
   movers hand-checked, notes beside `PINNED_TRIAGE_PRICING`).

6. **2026-08-01, sixth capture — after the rating billing-domain review fix
   wave** (T-D-23…T-D-32 + the #22 traceability conversion). Document movement
   only, no checker change. **Live findings 6 → 5**: rating's
   `P2/traceability-convention-unknown` coverage statement is gone because all
   16 rating slices now open §5 with a `**Traces to**:` block — 43 FRs checked
   per-id, all single-owner on the first pass (a live finding legitimately
   resolved by adopting the convention, not suppressed). Suppressed unchanged
   at 68 (no pinned-debt member is rating-side). One test retired into two:
   the rating half of `test_rating_and_subscriptions_report_convention_unknown…`
   became a positive full-coverage assertion for rating.

7. **2026-08-01, seventh capture — after the subscriptions wave-3 review fix
   wave** (SUB-D-20…SUB-D-26 + the direct-fix tier). Document movement only, no
   checker change. **Live findings 5 → 2** — all three movers are the wave's
   #24h item, hand-checked against the diff: (i) SUB-D-15
   `propagation-uninterpretable` resolved — the citation re-shaped to the
   resolver grammar (`S3 §4.5; S8 §4.1 registry`) and slice 08's
   `SubscriptionRampHalted` registry row now cites SUB-D-15, so the claim
   verifies rather than merely parsing; (ii) SUB-D-16
   `propagation-unresolvable` resolved — the bare `SEAMS` target now reads
   `SEAMS **SUB-C1**`; (iii) subscriptions' `P2/traceability-convention-unknown`
   coverage statement gone — all 8 FR-bearing slices open §5 with a
   `**Traces to**:` block, 47 FRs checked per-id, all single-owner on the first
   pass (the same legitimate resolution as rating's sixth-capture conversion).
   Suppressed unchanged at 68 (no pinned-debt member moved). The two remaining
   live findings are the rating-side pair the wave did not own
   (`decision-register-unparsed` for rating's T-D id shape; rating design/15's
   blockless `RATED` code).

The discipline is unchanged either way: a finding appearing or disappearing here
is a real claim about the design set, and re-freezing must be a deliberate,
separately-justified commit — never a way to make a failing change look green.

8. **2026-08-02, eighth capture — after the D-139 docs wave.** Document movement
   only, no checker change. D-139 is pricing's adoption of rating T-D-25 (the
   `capacityCharge` covered-granule factor), found while authoring the `reserved`
   joint fixture rather than by a review pass. **The finding set did not move at
   all**: `live-text.txt` is byte-identical, and `live-show-known-debt.txt` differs
   only in line numbers — 21 DECISIONS.md findings shifted by one, from the single
   board row the decision added. Live findings unchanged, suppressed unchanged.
   The neighbourhood pins moved in the same commit: `multi-region` 16 -> 17 and
   `weak-coverage` 39 -> 38, one mover hand-checked per-id against the pre-wave
   tree in a detached worktree — `nfr-data-residency`, whose two DESIGN.md anchors
   straddle the D-72 register summary in §4 that D-139 lengthened. Total
   requirements unchanged.

9. **2026-08-02, ninth capture — after the D-140 docs wave.** Document movement
   only, no checker change. D-140 is the REST route-shape reconciliation — the
   design set's `/v1/pricing/{resource}` paths and three colon-suffixed custom
   methods were both denied by the workspace's `DE0801` lint, so no documented
   endpoint was implementable; the wave rewrote 167 path strings to
   `/bss-pricing/v1/…` with actions as sub-resource segments, added the decision,
   and stated the rule normatively once in `design/01-foundation.md` §3.3. Like
   the eighth capture, **the finding set did not move**: `live-text.txt` and
   `live-json.json` are byte-identical, and `live-show-known-debt.txt` differs
   only in line numbers — the same 21 `DECISIONS.md` findings shifted by one,
   from the single board row the decision added. The neighbourhood pins moved in
   the same commit, in the opposite direction to the eighth capture:
   `multi-region` 17 -> 16 and `weak-coverage` 38 -> 39, one mover
   (`fr-level-aggregation`) hand-checked per-id against the pre-wave tree in a
   detached worktree. Its cause is the fixed window grid rather than any text
   about it: adding one board row to the pre-wave tree and changing nothing else
   reproduces that mover exactly and no other (controlled run, notes beside
   `PINNED_TRIAGE_PRICING`). Total requirements unchanged at 77.

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
