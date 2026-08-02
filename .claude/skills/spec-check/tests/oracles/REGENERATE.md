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

10. **2026-08-02, tenth capture — after the D-141…D-148 docs wave.** Document
    movement only, no checker change. This is the **second implementation-side
    wave** and the first raised not by a lint refusing to build the documents
    (that was D-140) but by writing Group **G3**, the gear's draft-authoring
    plane, *against* them: fifteen places where the code needed a rule the design
    set did not state, twelve of which became eight decisions — D-141 the
    per-price-row ETag presented by every draft mutation including `DELETE`;
    D-142 the two-state dedup row with expiry evaluated at claim time and before
    the payload digest; D-143 `IDEMPOTENCY_KEY_IN_FLIGHT` (409); D-144 instants
    UTC at millisecond resolution; D-145 `revision` as an identity with a
    terminal `abandoned` state; D-146 two codes narrowed out of
    `LIFECYCLE_FORBIDDEN`; D-147 `grandfatherUntil` confined to grandfathered
    rows; D-148 a draft-plane partial `UNIQUE` on the canonical scope key.

    Like the eighth and ninth captures, **the finding set did not move**:
    `live-text.txt` and `live-json.json` are byte-identical, and
    `live-show-known-debt.txt` differs only in line numbers — the same 21
    `DECISIONS.md` findings, every one shifted **uniformly +8**, one per
    status-board row the eight decisions added. Live findings unchanged at 2,
    suppressed unchanged at 68; `--gear gears/bss/pricing/docs --auto-context`
    still reports 0 live and 68 suppressed, so no pinned-debt member was paid
    down and none was added.

    The neighbourhood pins moved in the same commit, and this time the headline
    arithmetic hid compensating moves: **seven** movers net to +3
    `multi-region`, −1 `anchored:no-account`, −2 `weak-coverage`, judged 55 → 56,
    total unchanged at 77. Every one was diffed per-id against the pre-wave tree
    (`3b6fa985`) in a detached worktree and hand-checked against the wave's own
    edits; the full per-id record is beside `PINNED_TRIAGE_PRICING`. In short:
    `fr-bulk-price-import`, `fr-concurrent-edit`, `fr-mutation-idempotency` and
    `fr-pricewindow-coverage` gained genuine second accounts the wave wrote (the
    rewritten S1 §3.7 `pricing_price` / `pricing_idempotency_dedup` bullets and
    the D-141/D-142/D-143 register entries, two of which name their requirement
    id verbatim); `fr-published-rows-append-only` gained two accounts that are
    **register-digest prose** rather than rule statements, so a judge should be
    expected to answer `mentions`. The two losses were the ones worth the check,
    and neither is a document defect: `fr-grandfathering-eligibility` fell two
    steps to `anchored:no-account` because D-147 grew its *declaration* 53 → 93
    terms while its two accounts matched **more** terms than before in absolute
    count (36 → 45, 34 → 45) — the documented terse-prose recall limitation, with
    D-147 verifiably propagated to `design/07-pricewindow-linkage.md` 236/300/343
    and P1 silent; and `nfr-mutation-latency` fell one step purely on the fixed
    window grid — its lost region is byte-identical, shifted +20 lines off a
    `6k+1` boundary by D-144's insertion, and inserting 20 **blank** lines at that
    same point in the pre-wave tree reproduces the mover exactly and no other.

11. **2026-08-02, eleventh entry — the D-143 veto-status edit. Nothing was
    re-captured.** This entry exists to say so. The edit records the 2026-08-02
    veto round in the pricing register — D-143 CONFIRMED as decided against
    block-and-replay, and the `abandon` endpoint D-145 implies put to the owner
    in the same round and kept — as a status-board flip, a preamble paragraph, two
    entry-body provenance additions and the matching `DESIGN.md` §4 sentence. Every
    one is an edit *within* an existing line, so the wave added no line and **not
    even a line number shifted**: all three oracle files are byte-identical to the
    tenth capture and were left untouched. `--gear gears/bss/pricing/docs
    --auto-context` reports 0 live and 68 suppressed, unchanged.

    **The triage pin moved anyway, and that is the whole point of this entry** —
    "the oracles did not move but the pin did" is the case the next person will
    want to have been told about, because the two layers answer different
    questions and a byte-identical stdout is no evidence at all about N1.
    `suspicious:multi-region` 19 → 20 and `suspicious:weak-coverage` 36 ← 37,
    judged unchanged at 56 (both classes are judged, so the swap is
    judge-neutral), total unchanged at 77. **One mover**, `fr-tax-display-basis`,
    weak → multi, isolated to **one term**: its declaration says "not the plan as a
    **whole**", the register's status-paragraph window already carried 60 of its 101
    discriminating terms (0.594, one term under the 0.6 threshold), and the new
    veto-round sentence says "preserves the **MUST** whole" — 61/101, 0.604, over.
    Controlled in both directions against the pre-edit tree (`eb10a408`) in a
    detached worktree: applying the patch reproduces 20 / 36 and no other mover;
    changing that single word and nothing else restores 19 / 37. It is a
    fraction-threshold artifact on a common English word, not a second account —
    the status paragraph says nothing about tax display basis. The per-id record,
    and the reason the wording was left as written rather than tuned to the
    scorer, are beside `PINNED_TRIAGE_PRICING`.

12. **2026-08-02, twelfth entry — the D-145 amendment. Nothing was re-captured.**
    This entry exists to say so, and to record the second case of "the oracles did not
    move but the pin did". The edit amends D-145 in place: its "a new draft opens
    immediately" is true of every plan except one that has **never published**, whose
    only revision is `abandoned` — `create_draft` writes revision `0` literally and
    `open_revision` needs a current revision to succeed from, so that plan holds no
    current revision, no open draft and no route to either, and the id is spent. The
    owner kept the state and made the refusal honest, so the amendment mints one
    Foundation-owned code, **`PLAN_ABANDONED_NO_SUCCESSOR` (422)**, against the two
    rejected alternatives (minting `max(revision) + 1` on the create path, which turns a
    retried create into a silent second revision of an existing plan; and exempting
    revision `0` from the identity rule, which is the unstable name D-145 removes).

    All three oracle files are **byte-identical** to the tenth capture and were left
    untouched. Two things had to hold for that, and both did. The entry's ~66 added lines
    sit at `DECISIONS.md:1220` and in `design/01-foundation.md` §3.3/§4.3, `design/02`
    §4/§5 and `DESIGN.md` §4 — every pinned `P1/propagation-missing` anchor is at
    `DECISIONS.md:632` or earlier, so no finding's line number moved; and the new code is
    declared inside `design/01-foundation.md`'s `**Problem responses (RFC 9457):**` block
    **and** referenced outside a block (§4.3, `design/02` §4/§5, the register, the DESIGN
    digest), so `P3/code-unreferenced` gains no member and the pinned unreferenced-code
    list is untouched. `--gear gears/bss/pricing/docs --auto-context` reports 0 live and
    68 suppressed, unchanged.

    **The triage pin moved, on a cause with nothing to do with the amendment's subject.**
    `anchored:no-account` 15 → 14 and `suspicious:multi-region` 20 → 21, judged 56 → 57,
    `weak-coverage` and the total unchanged (36, 77). **Six movers**, and all six are the
    same corpus-global `DF_CUTOFF = 0.25` swap: `catalog` fell to 0.24999 **without its
    window count changing at all** (568 both times — the corpus gained 11 windows, none
    carrying the word) and became discriminating for 29 requirements, while `rule` rose to
    0.25132 (565 → 571 windows, the amendment's own prose) and stopped being
    discriminating for 12. Both terms were inside 0.0013 of the cutoff before the edit.
    Recomputing both trees with those two terms dropped from every requirement's term set
    reproduces the **old** pin on both, with zero per-id differences — the full per-id
    arithmetic and the controlled run are beside `PINNED_TRIAGE_PRICING`.

13. **2026-08-03, thirteenth capture — after the D-149…D-154 docs wave.** Document
    movement only, no checker change. This is the **third implementation-side wave**,
    raised by building Group **G4** — the *shape of a plan*: Slice 2's four validator
    sets and its three revision-scoped child tables — against the documents. Where G3
    (the tenth capture) found rules the set did not state, G4 found rules it **states
    and cannot enforce**: §5 naming no code for a §3 requirement, or a §6 `CHECK`
    standing where a rule belongs. Six decisions — D-149 the cycle-shape step's four
    codeless requirements plus the undeclared `billing_cycle` that made the whole step
    vacuous (`BASE_MARKET_INCOMPLETE`, `CYCLE_METADATA_MISSING`); D-150 the add-on
    rule's three quantity bounds (`ADDON_QTY_RANGE_INVALID`); D-151 `displayTrialDays`
    bound to the `trial` kind (`DISPLAY_TRIAL_DAYS_INVALID`); D-152 the per-tenant
    configurables' carrier, `pricing_policy_object`; D-153 the price row's draft-plane
    transition guard; D-154 the resolved effective `taxCategory` frozen with the row —
    plus four mechanical fixes carrying no id.

    **This is the first capture since the seventh whose finding set moved, and it moved
    in the paying-down direction.** Live findings unchanged at **2** (both rating-side,
    untouched here). Suppressed **68 → 59**: nine pinned `P3/code-unreferenced` members
    left at once, which is the largest single pin payment since the pin was taken —
    `ADDON_CYCLE`, `ADDON_INCOMPATIBLE`, `DESCRIPTOR_INCOMPLETE`, `HYBRID_INCOMPLETE`,
    `PHASE_DURATION_INVALID`, `PLANTIER_DIVERGENT`, `PURCHASE_QTY_RANGE_INVALID`
    (design/02), `TAX_BASIS_INCOMPLETE` (design/04) and `BILLING_TIMING_MISSING`
    (design/06). The reason it is nine at once and not a suspicious sweep: the wave
    rewrote the four Slice-2 algorithms and Slice 4's tax-persist/policy steps, which
    is exactly where eight of the nine are raised, so the debt was payable in passing.
    Each was hand-checked in the working tree, and where the wave's prose had only
    *mentioned* a code from a neighbouring rule or from a register entry, a one-clause
    fix was applied so the **rule that raises it** names it — five such corrections,
    itemised beside `PINNED_UNREFERENCED_CODES_2026_07_29`. `PLANTIER_MISSING` and
    `SETUP_ROW_INVALID` sit in the same blocks and were **left pinned** deliberately:
    no rule of this wave raises either, and naming them would be tuning the documents
    to the measurement.

    Everything else in `live-show-known-debt.txt` is intact: all 21 `P1` members
    reproduce, every one shifted **uniformly +6**, one per status-board row the six
    decisions added. `live-text.txt` differs only in the suppressed count.

    The neighbourhood pins moved in the same commit, further than any previous wave
    and for three separable reasons: `no-region` 6 → 5, `multi-region` 21 → 28,
    `weak-coverage` 36 → 30, judged 57 → 58, total unchanged at 77. **Fourteen movers**,
    each diffed per-id against the pre-wave tree (`d5e18846`) in a detached worktree:
    four are accounts the wave genuinely wrote (`fr-billing-cycles`,
    `fr-billing-descriptors`, `fr-plan-phases`, `fr-plantier-mandatory`), six gained
    only the D-72 register digest at 0.600–0.636 (the `mentions` class the tenth
    capture named — including `nfr-size-limits`, which gained its first candidate
    region ever and is the whole of the `no-region` move), and **four are the
    `DF_CUTOFF = 0.25` artifact on one term**, `catalog`, which was sitting at exactly
    568/2272 = 0.25000 and is now 573/2290 = 0.25022. A controlled run holding that one
    term constant on both sides — excluded at 0.249 or admitted at 0.2503 — removes all
    four and reproduces nothing else. Full per-id record beside `PINNED_TRIAGE_PRICING`.

14. **2026-08-03, fourteenth capture — after the D-155…D-161 docs wave.** Document movement
    only, no checker change. This is the **fourth implementation-side wave**, raised by
    building Group **G5** — the **publish commit**: the pipeline re-run inside the commit
    transaction, the lifecycle flips, the transactional outbox, the fail-closed
    `CatalogVersion` request and the segmented audit chain. Where G3 (tenth capture) found
    rules the set did not state and G4 (thirteenth) found rules it states and cannot enforce,
    G5 found the set **contradicting itself** and **promising values with no producer**. Seven
    decisions — D-155 the commit flipping exactly the `(price_id, row_version)` set its
    re-validation judged, with the input enumeration and the Slice-7 premise; D-156 the
    `CatalogVersion` request inside the transaction, after re-validation and before the writes;
    D-157 the pending-ref row's `(subject_kind, subject_ref)`; D-158 the audit log's two
    declared vocabularies; D-159 `CONCURRENT_MUTATION` (409); D-160 the code-carrying advisory
    channel and its two codes; D-161 the snapshot stamp's absent third segment, forked to the
    owner — plus two mechanical fixes carrying no id.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are byte-identical
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly
    +7**, one per status-board row the seven decisions added. Live findings unchanged at 2
    (both rating-side), suppressed unchanged at 59; `--gear gears/bss/pricing/docs
    --auto-context` still reports 0 live and 59 suppressed, so no pinned-debt member was paid
    down and none was added. The three new codes were each declared inside
    `design/01-foundation.md`'s `**Problem responses (RFC 9457):**` block **and** referenced
    outside a block — `CONCURRENT_MUTATION` at §3.7 (`pricing_audit_log`, `pricing_outbox`)
    and `design/05` §6, `TIER_BAND_PRICE_INCREASE` at `design/03` `inst-tb-order`,
    `PLAN_SIZE_SOFT_CAP_EXCEEDED` at §1.2 and PRD §7.1 — so `P3/code-unreferenced` gains no
    member.

    **The triage pin moved further than any previous wave: `anchored:no-account` 14 → 10,
    `multi-region` 28 → 40, `weak-coverage` 30 → 22, judged 57 … 58 → 62, `no-region` and the
    total unchanged (5, 77).** **Fourteen movers**, every one diffed per-id against the
    pre-wave tree (`a26991b8`) in a detached worktree, and they split into three causes with a
    controlled run separating them exactly:

    - **Three are the `catalog` document-frequency artifact**, the same term as the twelfth
      entry and the thirteenth capture, oscillating around `DF_CUTOFF = 0.25` for the third
      time: 573/2290 = 0.25022 (rejected) → 580/2324 = 0.24957 (admitted) — its window count
      **rose** while the corpus grew faster. It is the **only** term in the corpus that crossed
      the cutoff in either direction (checked exhaustively over all 4,809 terms).
      `fr-catalogversion-increment` (weak → multi) and `fr-event-contract` (no-account → weak)
      gained **no new region at all** — their existing regions merely crossed
      `SCORE_THRESHOLD = 0.6` (0.579/0.632 → 0.600/0.650 and 0.455/0.591 → 0.478/0.609) — and
      `fr-model-kind-conformance` gained two regions at exactly 0.611 for the same reason.
      Neutralising that one term on both trees removes all three and reproduces nothing else.
    - **Two are accounts this wave genuinely wrote**, and both are requirements the wave is
      about. `nfr-size-limits` (weak → multi) gains `DECISIONS.md:1345-1356` at 0.657 — **D-160's
      entry**, which gives its ratified soft caps the advisory code they had never had — plus
      the wave preamble at 0.643; its own PRD declaration was rewritten in the same wave.
      `fr-pricing-snapshot` (no-account → multi) gains `DESIGN.md:505-516` at 0.771,
      `DECISIONS.md:19-30` at 0.747 and `DECISIONS.md:1345-1356` at 0.699 — **D-161's entry**,
      which is the account of what the catalog-side stamp contains. Its two id-anchored regions
      fell 0.538 → 0.253 in the same move, the documented terse-prose recall effect running in
      the usual direction: its declaration grew from one sentence to a paragraph, so the
      discriminating-term set grew and the unchanged anchors match a smaller fraction of it.
    - **Nine gained register prose and nothing else** — the `mentions` class the tenth capture
      named, at 0.600–0.646: `fr-addon-rules`, `fr-billing-timing`, `fr-customer-group-pricing`,
      `fr-grandfathering-eligibility`, `fr-level-aggregation`, `fr-migration-safety`,
      `fr-plan-change-contract`, `fr-plan-retirement`, `fr-trailing-tier-qualification`. Every
      new region of all nine is either `DESIGN.md` §4 (the D-72 register digest, windows
      499-510/505-516) or the `DECISIONS.md` "How to use this document" preamble (windows
      19-30/25-36). **Not one of the nine gained a region in a design slice**, so no slice edit
      of this wave created an account for any of them; what grew is the two documents a wave is
      *obliged* to grow — D-72 requires the digest to stay current, and the preamble records
      each wave. A judge should be expected to answer `mentions` for all nine.

    The four ids that entered the judged set are `fr-customer-group-pricing`,
    `fr-event-contract`, `fr-grandfathering-eligibility` and `fr-pricing-snapshot`, all
    promoted out of `anchored:no-account`; the +12/−8 between `multi-region` and
    `weak-coverage` is judge-neutral, both classes being in `JUDGED`. Nothing was reworded to
    move a score back, in either direction. Per-id record beside `PINNED_TRIAGE_PRICING`.

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
