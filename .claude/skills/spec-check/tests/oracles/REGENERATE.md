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

15. **2026-08-03, fifteenth capture — after the D-162 docs edit.** Document movement only, no
    checker change. D-162 is the product owner's answer to the §F.1 fork D-161 opened one wave
    earlier: `pricingSnapshotRef`'s evaluation-policy segment is a **vocabulary generation**,
    `ep-<n>`, a declared constant of the gear naming which evaluation-policy field set a
    snapshot's frozen row content is read under. Writing it required writing down the field set,
    which the design set never had — the phrase runs through it sixty-odd times unexpanded and is
    expanded once, in the PRD glossary, as three fields three later decisions added to — so the
    edit declares a **nine-field roster**, what sits outside it and why, and an **append-only
    generation log** the gear's build replays, since a generation nobody remembers to bump
    asserts on posted money a stability that is not there.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are byte-identical
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly
    +1**, the single status-board row the decision added. Live findings unchanged at 2 (both
    rating-side), suppressed unchanged at 59; `--gear gears/bss/pricing/docs --auto-context`
    still reports 0 live and 59 suppressed. **No wire code was minted**, which is the point of
    a generation: the segment is a value, not a refusal, so `P3/code-unreferenced` has nothing
    to gain or lose. One near-miss is worth recording because it was caught rather than
    committed — a draft of the register entry spelled `EVAL_POLICY_MISPLACED` while arguing
    why `quantity_source` sits outside the roster, which **paid down a pinned
    `P3/code-unreferenced` member** (59 → 58) without any rule having started to raise it. The
    sentence was reworded to name the response rather than the token. A pin paid by a register
    mention is a pin paid by nothing.

    **The triage pin moved: `no-region` 5 → 4, `anchored:no-account` 10 → 9, `multi-region`
    40 → 44, `weak-coverage` 22 → 20, judged 62 → 64, total unchanged at 77.** **Six movers**,
    each diffed per-id against the pre-edit tree (`e3fcd727`) in a detached worktree, and for
    the first time in four waves there is **no `DF_CUTOFF` component at all**: checked
    exhaustively over the union of both trees' terms (4,809 → 4,835), **zero** crossed the
    cutoff in either direction, and `catalog` — the term behind the twelfth entry and the
    thirteenth and fourteenth captures — moved 0.24957 → 0.24797, further inside rather than
    across.

    All six are region gains with **one** cause. Every new region of every mover is either
    `DESIGN.md` §4 (the D-72 register digest, windows 499-510 / 505-516) or the `DECISIONS.md`
    preamble (19-30): `nfr-read-latency` no-region → weak at 0.643 (its first candidate region
    ever), `fr-bundle-composition` no-account → weak at 0.607, `fr-custom-frequency` at 0.621,
    `fr-package-pricing` at exactly 0.600, `fr-invoice-currency-binding` at exactly 0.600, and
    `fr-consumer-readmodel-resolution` at 0.638 + 0.606. Only the last has a claim to being a
    real account — its declaration names "model kind, ordered tier bands, and evaluation-policy
    fields", and the digest paragraph is the first text in the set that says which fields those
    are. **Not one mover gained a region in a design slice, in the PRD, or in the new normative
    §4.4 block**, which is where all of the edit's substance went.

    Two controlled runs settle it, and they are the cleanest separation any capture has had.
    (i) Pre-edit tree plus **only** the new status-board row: **zero** movers, so none of this
    is the fixed window grid. (ii) Pre-edit tree with **only** `DESIGN.md` and `DECISIONS.md`
    replaced: reproduces all six movers in the same directions and nothing else. The judged
    share is now 64/77 = 83% and this is the fourth consecutive wave to grow it without a
    design slice moving — standing rule 4's concern, now demonstrated outright rather than
    inferred. Noted beside `PINNED_JUDGE_CALLS`; nothing was reworded to move a score in either
    direction.

16. **2026-08-03, sixteenth capture — after the D-163…D-168 docs wave.** Document movement
    only, no checker change. This is the **fifth implementation-side wave**, raised by building
    Group **G6** — the **read side**: turning a pending `CatalogVersion` handle into a committed
    number, projecting the frozen per-subject read model, advancing the pin frontier, and
    running the degraded path. Where G3 (tenth capture) found rules the set does not state, G4
    (thirteenth) found rules it states and cannot enforce, and G5 (fourteenth) found it
    contradicting itself and promising values with no producer, **G6 found rules decidable only
    under premises nobody had written down, and clauses that cannot be satisfied with what
    exists.** Six decisions — D-163 batch atomicity as the registry's contract, the bound on
    when the projector may call a version complete, and the straggler refusal whose price is
    stated with the guarantee; D-164 pin-eligibility per tenant and the frontier's forward walk;
    D-165 the ref row freezing the revision **and** the lifecycle state its own publish judged;
    D-166 one recorded instant and three disjoint publish-visibility predicates; D-167 what a
    delta carries before Slices 4/6/7/10 and what each absence owes; D-168 the Slice-6 contract
    pair stamped as a pair or not at all, its text forked to the owner — plus three mechanical
    fixes carrying no id.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are byte-identical
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly
    +6**, one per status-board row the six decisions added. Live findings unchanged at 2 (both
    rating-side), suppressed unchanged at 59; `--gear gears/bss/pricing/docs --auto-context`
    still reports 0 live and 59 suppressed, so no pinned-debt member was paid down and none was
    added. **No wire code was minted** — the wave's two refusals both live on the projector,
    which has no caller, so by D-146's own line a refusal there has nobody to report to and
    `P3/code-unreferenced` has nothing to gain or lose.

    **The triage pin moved: `anchored:no-account` 9 → 6, `suspicious:multi-region` 44 → 46,
    `suspicious:weak-coverage` 20 → 21, judged 64 → 67, `no-region` and the total unchanged
    (4, 77).** **Six movers**, every one diffed per-id against the pre-wave tree (`99523f15`) in
    a detached worktree, with four controlled runs separating them:

    - **One `DF_CUTOFF` artifact, on a new term.** `same` moved 0.24968 → 0.25126 and stopped
      being discriminating — the **only** term to cross in either direction, checked
      exhaustively over the 4,835 terms both trees share (`catalog`, the term behind the twelfth
      entry and the thirteenth and fourteenth captures, moved 0.24797 → 0.24537, further
      inside). Neutralising it on both trees removes `fr-one-time-setup` (multi → weak, having
      lost the `DESIGN.md` digest window at 0.606) and reproduces every other mover unchanged.
    - **Three gained register prose and nothing else** — `fr-min-qty-floor`,
      `nfr-mutation-latency` and `nfr-observability`, at 0.605–0.714, every new region either
      `DESIGN.md` §4 (the D-72 digest) or the `DECISIONS.md` preamble. The `mentions` class.
    - **One gained a genuine account**, and it is the one requirement the wave is about:
      `fr-publish-fanout-atomicity` (no-account → multi) gains D-166's entry at
      `DECISIONS.md:1399-1410` (0.743), the preamble (0.786) and the digest (0.657), while its
      two pre-existing regions **fell** (0.273 → 0.186, 0.545 → 0.400) because its own PRD
      declaration grew by a paragraph — the documented terse-prose recall effect.
    - **One is an old account tipped over by two words of someone else's rule.**
      `fr-supersession` (no-account → weak) gains §3.7's table-bullet window at **exactly
      0.600**, up from 0.582 on the *same* content: 64/110 discriminating terms → 66/110, and
      the two are **`batching`** and **`delay`**, from D-166's sentences in the
      `pricing_catalog_version_ref` and `pricing_pin_frontier` bullets. Neither says anything
      about supersession; what the window does say about it (`supersedes_price_id`, the
      published-plane partial `UNIQUE`, the `published → superseded` whitelist, the price-history
      bullet) is unchanged and was always sub-threshold.

    Two controlled runs settle the grid question and one settles the content question.
    (i) Pre-wave tree plus **only** the six new status-board rows: **zero** movers, so no part
    of this is the fixed window grid. (ii) Pre-wave tree with **only** `DESIGN.md` and
    `DECISIONS.md` replaced: five of the six, `fr-supersession` excepted. (iii) The post-wave
    foundation slice with **only** the two §3.7 bullets reverted *in place* — each is a single
    line, so every line number stays identical — **zero** movers, which is what proves
    `fr-supersession` is content and not grid.

    **Not one mover gained a region in a design slice as an account of itself.** The judged
    share is now 67/77 = 87% and this is the **fifth** consecutive wave to grow it that way;
    the previous four are the tenth and thirteenth-to-fifteenth entries above. It is recorded
    as a measurement-health signal, not tuned away — nothing was reworded in either direction,
    and `batching` and `delay` are the right words for the sentences they are in.

17. **2026-08-03, seventeenth capture — after the D-169 register edit.** Document movement
    only, no checker change. D-169 is the product owner's answer to the §F.1 fork D-168 opened
    the same day, and it is a **removal**: `crossBoundaryWarningText` leaves the Slice-6
    consumer contract, the catalog publishes the K3 marker
    (`crossBoundaryChangePolicy = cancel_plus_new`) alone, and the surface that renders the
    warning owns its wording. The answer turned on a fact the fork statement did not have —
    `PRD.md` **AC #66** already required the preview/migration UI to warn that in-place credit
    is forfeited and to take an explicit confirmation, so the field was a *second* home for a
    sentence this set had already placed on the surface that shows it, and the second home is
    the one with an INSERT-only ≥ 7-year store behind it. It was recorded as its own decision
    rather than as a clause on D-168, following the D-161 → D-162 precedent one wave earlier.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are byte-identical
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly
    +1**, the single status-board row the decision added, and the member set is identical
    modulo those numbers. Live findings unchanged at 2 (both rating-side), suppressed unchanged
    at 59; `--gear gears/bss/pricing/docs --auto-context` still reports 0 live and 59
    suppressed. **No wire code was minted or retired** — a field left a read model, which is a
    value and not a refusal, so `P3/code-unreferenced` has nothing to gain or lose. The
    fifteenth entry's near-miss was watched for and did not recur: nothing in this edit names
    a pinned unreferenced code, because a pin paid down by a register mention rather than by
    the rule that raises it is a pin paid by nothing.

    **The triage pin moved: `anchored:no-account` 6 → 5, `suspicious:multi-region` 46 → 49,
    `suspicious:weak-coverage` 21 → 19, judged 67 → 68, `no-region` and the total unchanged
    (4, 77).** **Four movers**, each diffed per-id against the pre-edit tree (`8fb20548`) in a
    detached worktree. There is **no `DF_CUTOFF` component for the second edit running**:
    checked exhaustively over the 4,914 terms both trees share, **zero** crossed the cutoff in
    either direction (`same`, the term behind the sixteenth capture, went 0.25126 → 0.25105 and
    stayed out; `catalog` 0.24537 → 0.24685 and stayed in).

    All four are region gains, and this is the cleanest — and most uncomfortable — shape any
    capture has recorded. **Each mover gained exactly one region; each is the D-72 register
    digest or the register preamble; each crossed `SCORE_THRESHOLD = 0.6` on exactly one term;
    and every other region of every mover is byte-identical, same window, same score, same
    matched-term count.** `fr-per-seat` (no-account → weak) gained `DESIGN.md:499-510` at
    16/26 = 0.6154 on the word **`wording`**; `fr-one-time-setup` (weak → multi) the same
    window at 20/32 = 0.6250 on **`preview`**; `fr-reserved-capacity` (weak → multi)
    `DESIGN.md:505-516` at 35/58 = 0.6034 on **`authorable`**; `fr-plan-clone` (weak → multi)
    `DECISIONS.md:19-30` at 26/43 = 0.6047 on **`going`**. Two of the four terms come from one
    clause about one *rejected* option ("re-authorable going forward only"), and not one of the
    four says anything about per-seat pricing, one-time setup fees, reserved capacity or plan
    cloning.

    Three controlled runs separate it exactly. (i) Pre-edit tree plus **only** the `DESIGN.md`
    D-72 digest sentence: the three `DESIGN.md` movers and nothing else. (ii) Pre-edit tree
    plus **only** the `DECISIONS.md` preamble line: `fr-plan-clone` and nothing else. (iii) The
    post-edit tree with the digest and the preamble reverted, so the D-169 entry, the board
    row, the struck §F.1 row, the five Slice-6 edits and the two PRD edits are all present:
    **zero** movers, histogram identical to the pre-edit pin.

    **The judged share is now 68/77 = 88%, the sixth consecutive wave to grow it without a
    design slice creating an account for any mover** — and the smallest edit yet to move it.
    That is the signal worth carrying forward rather than tuning away: a digest D-72 *obliges*
    every wave to keep current summarises every requirement in the gear, so every requirement's
    vocabulary can match it, and the fixed 12-line window over that one enormous line now sits
    within a hundredth of the threshold for a large part of the corpus. Nothing was reworded in
    either direction; `wording`, `preview`, `authorable` and `going` are the right words for
    the sentences they are in. Per-id record beside `PINNED_TRIAGE_PRICING`.

18. **2026-08-03, eighteenth capture — after the D-170…D-174 docs wave.** Document movement
    only, no checker change. This is the **sixth implementation-side wave** and the one that
    closes Phase 2, raised by building Group **G7** — the gear's **REST surface**: the nine
    authoring routes, their preconditions, their authz gate and the `OpenAPI` registration a
    client is generated from. Where G3 (tenth capture) found rules the set does not state, G4
    (thirteenth) rules it states and cannot enforce, G5 (fourteenth) the set contradicting
    itself and promising values with no producer, and G6 (sixteenth) rules decidable only
    under premises nobody had written down, **G7 found the set stating a contract and never
    stating its transport.** Five decisions — D-170 what a plan route addresses and the
    revision-qualified tag that names it; D-171 `If-Match` and `Idempotency-Key`, the two
    header names the set had never written down; D-172 the spent-plan refusal's arm list
    corrected (the create names no plan id, the read answers 404); D-173 one facet per `PATCH`
    with the multi-facet operation named as undesigned; D-174 the idempotency digest taken over
    the parsed request rather than the received bytes — plus three mechanical fixes carrying
    no id.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are byte-identical
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly
    +5**, one per status-board row the five decisions added, and the member set is identical
    modulo those numbers. Live findings unchanged at 2 (both rating-side), suppressed unchanged
    at 59; `--gear gears/bss/pricing/docs --auto-context` still reports 0 live and 59
    suppressed, so no pinned-debt member was paid down and none was added. **No wire code was
    minted** — every refusal this wave decides is one the set already declares (`STALE_VERSION`
    for a tag naming the wrong revision, `PLAN_ABANDONED_NO_SUCCESSOR` on three arms instead of
    four, and the validation envelope's codeless 400 for an absent header, a second facet and
    an undecodable cursor) — so `P3/code-unreferenced` has nothing to gain or lose. The
    fifteenth entry's near-miss was watched for and did not recur: nothing in this wave names a
    pinned unreferenced code, because a pin paid down by a register mention rather than by the
    rule that raises it is a pin paid by nothing.

    **The triage pin moved: `no-region` 4 → 3, `suspicious:multi-region` 49 → 52,
    `suspicious:weak-coverage` 19 → 17, judged 68 → 69, `anchored:no-account` and the total
    unchanged (5, 77).** **Four movers**, each diffed per-id against the pre-wave tree
    (`f8f3ed51`) in a detached worktree. There is **no `DF_CUTOFF` component for the third edit
    running**: checked exhaustively over the 4,923 terms both trees share, **zero** crossed the
    cutoff in either direction, and the two terms that have oscillated around it in earlier
    captures both moved further inside (`catalog` 0.24685 → 0.24626, `same` 0.25105 → 0.25707).
    All four are region gains, and four controlled runs split them **3 + 1**:

    - **Three gained the D-72 digest or the register preamble and nothing else** — the
      `mentions` class. `fr-approval-threshold-policy` (weak → multi) gains `DESIGN.md:499-510`
      at 82/138 = 0.594 → 86/138 = 0.623 on `direction`, `routes`, `multi` and `regardless`;
      `fr-tenant-brand-isolation` (weak → multi) gains both summary windows on `authz` and
      `mutating`, against a **22-term** declaration where one term is worth 0.045; and
      `fr-priceoverlay-authoring` (weak → multi) gains both on `stack`, `discards`, `layer`,
      `strictly` and `tariffs`. The last is the only mover of the four whose new region says
      anything about its own subject — both windows carry the wave's standing-list correction,
      which *is* about the overlay stack — but what it says is a **status fact about a fork**
      (D-138 closed the `fixed` arithmetic; the sort direction survives as its own row), not a
      statement of the authoring rule, so `mentions` remains the expected verdict.
    - **One is a design slice, and it is the whole of the judged-share increment.**
      `nfr-event-propagation` (no-region → weak) gains `design/01-foundation.md:487-498` at
      11/17 = 0.647, up from 10/17 = 0.588 on the **same window with the same line numbers**.
      **One term: `within`**, from D-174's *rejected-alternative* clause quoting §3.3's
      "additive-only **within** a major version", against the requirement's own "MUST reach
      downstream consumers **within** 5 seconds at p95". The window's other ten matched terms
      are `pricing_outbox`'s bullet — frozen event names, dedup/correlation keys,
      `(tenantId, aggregateId)` ordering — which is genuinely this requirement's neighbourhood,
      is unchanged by this wave, and was sub-threshold before it. D-174 created no account; it
      tipped an old one over 0.6 on a preposition.

    Four controlled runs settle it. (i) Pre-wave tree with **only** `DESIGN.md` and
    `DECISIONS.md` replaced: the three summary movers and nothing else. (ii) Pre-wave tree with
    everything **except** those two replaced — the propagation edits in three design slices and
    the PRD: `nfr-event-propagation` and nothing else. (iii) Post-wave tree with the D-174
    sentence removed **in place** from the `pricing_idempotency_dedup` bullet (one enormous
    line, so every line number stays identical): `nfr-event-propagation` reverts, which is what
    proves it is content and not the grid. (iv) Pre-wave tree with **26 blank lines** inserted
    at §3.3 — the exact shift this wave puts above §3.7, with no content at all: **zero** of the
    four movers, so the grid contributes nothing to any of them; its one effect of its own is
    `fr-supersession` weak → no-account, which the wave's real content cancels, that requirement
    having now sat within a hundredth of the threshold on the same §3.7 window for two
    consecutive waves on terms that say nothing about supersession.

    **The judged share is now 69/77 = 90%, the seventh consecutive wave to grow it — and the
    first whose increment is not the digest or the preamble.** The three weak → multi moves are
    judge-neutral, both classes being in `JUDGED`, so the entire +1 arrives from a normative
    slice. That is a change in the *shape* of the signal rather than in its direction: the cause
    is still not an account, but the vehicle is no longer only the two documents a wave is
    obliged to grow. Recorded as a measurement-health signal, not tuned away — nothing was
    reworded in either direction, and `within`, `authz`, `direction` and `stack` are the right
    words for the sentences they are in. Per-id record beside `PINNED_TRIAGE_PRICING`.

19. **2026-08-03, nineteenth capture — after the D-175…D-178 docs wave.** Document movement
    only, no checker change. This is the **seventh implementation-side wave** and the first
    raised by a group whose purpose was **closing** the register's owed-back clauses rather than
    building a plane: Group **G8** built six waves of follow-through (761 → 847 tests; five audit
    writers where there had been one; the four plan-shape rules, the soft-cap advisory, the
    contention refusal, the degraded instant, the completeness bound, the cross-boundary marker),
    and closing them is what surfaced what the design set owed back. Where G3 (tenth capture)
    found rules the set does not state, G4 (thirteenth) rules it states and cannot enforce, G5
    (fourteenth) the set contradicting itself, G6 (sixteenth) rules decidable only under unwritten
    premises and G7 (eighteenth) a contract with no stated transport, **G8 found the set's own
    accounts of itself untrue.** Four decisions — D-175 the three draft-authoring audit verbs
    `create`/`update`/`delete` plus the `action` vocabulary's closure rule ("no writer without a
    token", the companion of D-158's "no token without a writer"); D-176 a precondition evaluated
    **inside** the transaction that writes the mutation it guards; D-177 the two Slice-10
    primitives refused on every authoring path, with the refusal named load-bearing against the
    publish freeze; D-178 the correlation id's single producer — plus five mechanical corrections
    carrying no id, four of them corrections to the register itself.

    **The finding set did not move.** `live-text.txt` and `live-json.json` are **byte-identical**
    and were re-captured to no effect; `live-show-known-debt.txt` differs **only in line
    numbers** — the same 21 `P1/propagation-missing` members, every one shifted **uniformly +4**,
    one per status-board row the four decisions added, and the member set is identical modulo
    those numbers (verified by parsing both files and comparing with line numbers stripped). Live
    findings unchanged at 2 (both rating-side), suppressed unchanged at 59; `--gear
    gears/bss/pricing/docs --auto-context` still reports **0 live and 59 suppressed**, so no
    pinned-debt member was paid down and none was added. **No wire code was minted.** The wave
    names ten Slice-10 codes in `design/10-advanced-primitives.md` §3 and two Foundation codes in
    §3.3, and every one of them is already both declared and referenced by the rule that raises
    it — which the byte-identical `live-text.txt` proves rather than asserts. The standing caution
    held: a pin paid down by a prose mention rather than by the rule that raises it is a pin paid
    by nothing, and nothing here names a pinned unreferenced code.

    **The triage pin moved, and for the first time in eight captures it moved *down*:
    `no-region` 3 → 4, `anchored:no-account` 5 → 6, `suspicious:multi-region` 52 → 54,
    `suspicious:weak-coverage` 17 → 13, judged 69 → 67, total unchanged at 77.** **Four movers**,
    each diffed per-id against the pre-wave tree (`5a8c801e`) in a detached worktree, with
    **five** controlled runs. **One `DF_CUTOFF` component, the first in four edits, and it causes
    nothing:** over the 4,992 terms both trees share exactly one crossed the cutoff — `unit`
    0.25042 → 0.24661, moving *inside* — by **dilution** rather than usage, the pricing corpus
    going from 2,404 to 2,433 windows so every document frequency fell slightly and `unit` was
    sitting four ten-thousandths outside. The two historical oscillators did not cross: `catalog`
    0.24626 → 0.24127 (further inside), `same` 0.25707 → 0.26141 (further outside).

    The four split **2 + 2**:

    - **Two are the register preamble, the documented `mentions` class and judge-neutral.**
      `fr-per-seat` and `fr-price-history-export` each gain `DECISIONS.md:19-30` and nothing else
      — the preamble grew by a wave narrative that summarises every requirement in the gear, so
      any requirement's vocabulary can match it. Both weak → multi, both classes in `JUDGED`.
    - **Two are threshold-adjacent ids losing a window to the line grid, and they are the whole
      of the −2.** `nfr-event-propagation` weak → **no-region**: its only region was
      `design/01-foundation.md:487-498` at 11/17 = **0.6471**, and the best window §3.7's bullets
      now fall into scores 9/17 = **0.5294**. No DF component at all (`unit` is not among its 17
      terms) and no substantive change to its neighbourhood — `pricing_outbox`'s bullet is still
      its genuine account and this wave *extended* it — but 38 lines were added above §3.7 and a
      12-line window at step 6 re-slices, putting two of the eleven matched terms on opposite
      sides of a boundary. This **exactly reverses the eighteenth capture's promotion**, which
      arrived on the single preposition `within` from a rejected-alternative clause.
      `fr-supersession` weak → **anchored:no-account**: 67/110 = **0.6091** → 63/111 =
      **0.5676**. Because `unit` *is* in this id's terms the decomposition is stated: at the old
      denominator the new match count still fails (63/110 = 0.5727), and at the new denominator
      the old match count still passes (67/111 = 0.6036), so the grid is necessary **and**
      sufficient and the DF crossing is neither — 0.005 of a 0.041 fall.

    Five controlled runs settle it. (i) Pre-wave tree with **only** `DESIGN.md` and
    `DECISIONS.md` replaced: the two `mentions` gainers and nothing else. (ii) Pre-wave tree with
    everything **except** those two: the two departures and nothing else. (iii) Pre-wave tree with
    **38 blank lines** inserted at the two points this wave adds text to
    `design/01-foundation.md` — the exact shift it puts above §3.7, with **no content at all**:
    **both** departures reproduce exactly, so the grid is sufficient on its own. (iv) Pre-wave
    tree with only `design/01-foundation.md` replaced: the same two departures, so no other
    edited file contributes. (v) Pre-wave tree with the PRD, S2, S5, S10 and S12 replaced and S1
    left alone: **zero** movers — five edited design surfaces and a PRD edit move nothing
    whatever.

    **The judged share is 67/77 = 87%, ending seven consecutive waves of growth.** Said plainly:
    this wave's increment comes from **neither** a design slice **nor** the digest and preamble in
    any load-bearing sense. The two preamble gains are judge-neutral; the entire judged-share
    change is two ids losing a window they only ever held by a hundredth, because text was added
    *above* it. The eighteenth entry predicted this for both ids and named the mechanism — a
    blank-line shift reproduces it — so this is that prediction coming true rather than a new
    phenomenon. Nothing was reworded in either direction to chase the number and no threshold was
    touched. What a re-slice this fragile says is that a 12-line window at step 6 is a coarse
    instrument near 0.6; recorded as a measurement-health signal for whoever tunes it. Per-id
    record beside `PINNED_TRIAGE_PRICING`.

20. **2026-08-03, twentieth capture — after the phase-closing docs wave** (D-180, D-181, and two
    id-less records of what the phase proved by execution). Document movement only, no checker
    change. *(The D-179 wave between entry 19 and this one needed no capture: its edits landed in
    `design/01-foundation.md` §3.3 and every known-debt finding's line number is a `DECISIONS.md`
    line number.)*

    **Only `live-show-known-debt.txt` moved, and only in line numbers.** Normalising
    `DECISIONS.md:<n>` to a constant makes the two files byte-identical: 21 findings, the same 21
    findings, each shifted by the four rows this wave adds to the status board (three new/backfilled
    entries plus D-179's missing row). `live-text.txt` and `live-json.json` are byte-unchanged —
    **2 live findings, 59 known-debt**, exactly the pre-wave numbers.

    **The triage pin moved, and this is the largest move of any capture: `no-region` 4 → 3,
    `suspicious:multi-region` 54 → 59, `suspicious:weak-coverage` 14 → 10, judged 68 → 69, total
    unchanged at 77.** Six movers, each diffed per id against the pre-wave tree (`e7704c10`) in a
    detached worktree. **Every one of them is one new `term-overlap` region scoring 0.600–0.667,
    and every one of them lands in one of just two windows** — `DECISIONS.md:19-30`, which is the
    register's TOC plus its single-line **67 KB** preamble, and `design/01-foundation.md:541-552`,
    §3.7's bullet block at **29 KB**. Before the wave those six sat at 0.574–0.595 against the same
    windows.

    **The blank-line control is decisive, and it points the other way for the first time.** The
    pre-wave tree with 26 / 19 / 72 blank lines at this wave's three insertion points and no content
    at all reproduces the **old** histogram exactly. So this is not the grid re-slicing pre-existing
    text, as entries 18–20 were: the added words are the cause. What the words are settles it
    anyway. Term for term, the crossings are `primary` (`fr-bundle-composition`), `groups`/`many`
    (`fr-customer-group-pricing`), `applies`/`implicit`/`matrix` (`fr-model-kind`),
    `applies`/`execution`/`offered`/`void` (`fr-sellability-gate`), `events`
    (`nfr-event-propagation`) and `rating` (`nfr-publish-propagation`) — ordinary English from a
    wave about the audit `action` vocabulary and the correlation id, in windows that cite none of
    the six ids and say nothing about bundles, customer groups, model kinds or sellability.

    **One `DF_CUTOFF` crossing, the known oscillator, and it fully explains one mover.** Over the
    5,141 terms both trees share exactly one crossed: `unit` 0.24426 → 0.25070, moving *outside* —
    the same term entry 19 recorded moving *inside* at 0.25042 → 0.24661, oscillating on window
    dilution alone (2,481 → 2,501 windows). It touches `fr-customer-group-pricing`, where on
    `design/01-foundation.md:541-552` the matched count is **48 before and after** and the score
    goes 48/81 = 0.593 → 48/80 = 0.600: the threshold crossed by a shrinking denominator and
    nothing else. `catalog` 0.23700 → 0.23471 and `same` 0.25796 → 0.26190 did not cross.

    **What this adds to the model, and it is the sharpest measurement-health signal so far.**
    Entries 18–20 established that a 12-line window at step 6 is a coarse instrument near 0.6. This
    capture shows *why it is getting worse*: two documents have grown a paragraph-per-line habit, so
    two windows in the whole corpus each carry tens of thousands of characters and therefore a large
    fraction of **any** requirement's vocabulary. Both sit permanently within a term or two of the
    threshold for dozens of ids — four of the six moved on 1 term of 116, 3 of 39, 4 of 115 and 1 of
    17, and at 17 terms the score's own granularity (0.059) exceeds the distance travelled. Any wave
    that appends to the register preamble or to §3.7 will flip a handful of ids in whichever
    direction it pushes, and the count will keep climbing as those two paragraphs grow. Whoever
    tunes this should consider splitting a window on paragraph size rather than on line count.
    Nothing was reworded to move a score back (rule 6) and no threshold was touched. Per-id record
    beside `PINNED_TRIAGE_PRICING`.

21. **2026-08-04, twenty-first capture — after the D-182 docs wave** (the window plane's single
    register entry, and the first in that register written *before* the code it governs). Document
    movement only, no checker change.

    **Only `live-show-known-debt.txt` moved, and only in line numbers.** Normalising
    `DECISIONS.md:<n>` to a constant makes the two files byte-identical: 21 findings, the same 21
    findings, each shifted by **exactly +1** — the one status-board row this wave adds. Every
    known-debt finding's line number is a `DECISIONS.md` line number *above* the entry's own
    insertion point, so D-182's nine lines move nothing. `live-text.txt` and `live-json.json` are
    byte-unchanged — **2 live findings, 59 known-debt**, exactly the pre-wave numbers.

    **The triage pin did not move: 0 / 3 / 5 / 59 / 0 / 10 / 0, judged 69, total 77 — identical to
    entry 20's pin.** This is the first append to the register preamble since entry 20 named that
    window a permanent threshold-straddler, so the zero is worth as much as a move would be and was
    measured the same way: three **region** movers, **zero** triage movers, zero judge movers, each
    diffed per id against the pre-wave tree (`4d007405`) in a detached worktree.

    **The blank-line control separates all three exactly.** The pre-wave tree with 1 / 9 blank lines
    at this wave's two `DECISIONS.md` insertion points and **no content anywhere** reproduces two of
    the three movers byte-for-byte:

        id                        window before -> after                     control
        fr-prepaid-credit-grant   DECISIONS.md:547-558 -> 553-564            reproduced
        nfr-size-limits           DECISIONS.md:1357-1368 -> region dropped    reproduced
        fr-future-gap-coverage    design/07:205-216 -> 199-210               NOT reproduced

    So `fr-prepaid-credit-grant` and `nfr-size-limits` are the grid re-slicing pre-existing text on
    a +10-line offset — entries 18–20's artifact, with **zero** content contribution — and neither
    changes class: region count holds at 4 for the first, and the second's 3 → 2 stays
    `suspicious:multi-region` with two regions still clearing the threshold.
    `fr-future-gap-coverage` is this wave's **content**, and it is the honest one: the wave grows
    `inst-fg-trailing` in place, *both* the losing and the winning window contain that rule's line,
    the region count stays at 2 and the class stays `suspicious:weak-coverage` — the window
    re-centres on the rule that grew, in the one document the requirement is about. `design/07`
    gains no lines at all, which is why the control cannot reproduce it.

    **Why an append to the straddling window moved nobody, measured rather than assumed.**
    `DECISIONS.md:19-30` grew 66,743 → 68,440 bytes on its single-line preamble, and its region
    membership is **44 ids before, 44 after, 44 in the control — the same 44, none gained, none
    lost.** Entry 20's mechanism is unaffected and its warning stands; what this wave shows is the
    other face of it, that a window already carrying a large fraction of every requirement's
    vocabulary is also one that ordinary additions cannot push further. One thing was deliberately
    **not** done, unlike entry 20: no term-level `DF_CUTOFF` sweep was computed, because with zero
    triage movers there is no crossing to attribute. Nothing was reworded to hold a score (rule 6)
    and no threshold was touched. Per-id record beside `PINNED_TRIAGE_PRICING`.

22. **2026-08-05, twenty-second capture — after the D-183…D-193 phase-4 close docs wave.**
    Document movement only, no checker change. Eleven register entries — the first wave taken
    against the *implementation as built* rather than against the documents — plus a status-board
    paragraph and one clause each in S3 `inst-ps-supersede`, S5 (`inst-tp-distinct`, §5's
    threshold-policy row, §6's policy paragraph and the `materiality` column) and S7 (§5's window
    rows, §7's event list).

    **Only `live-show-known-debt.txt` moved, and only in line numbers.** Normalising
    `DECISIONS.md:<n>` to a constant makes the two files byte-identical: 21 findings, the same 21,
    each shifted by the eleven board rows this wave adds above them. `live-text.txt` and
    `live-json.json` are byte-unchanged — **2 live findings, 59 known-debt**, exactly the pre-wave
    numbers, and every one of the eleven new `**Propagated**` claims verifies against a real
    citation on the first run.

    **The triage pin moved, and this capture inverts entries 18–21's finding.** 3 / 5 / 59 / 10
    (judged 69) → 3 / **6** / 59 / **9** (judged **68**), total unchanged at 77. Five triage
    movers, each diffed per id against the pre-wave tree (`b65201b52`) in a detached worktree —
    and **the blank-line control reproduces none of them.** 121 blank lines in `DECISIONS.md` and
    2 each in S5/S7, no content anywhere, leaves the histogram byte-identical to pre-wave. Where
    entries 18–21 found the window grid re-slicing pre-existing text, every mover here is content
    or corpus statistics:

        id                       region that moved                        direction
        fr-future-gap-coverage   DECISIONS.md @ 0.625 gained              -> multi-region
        fr-scheduled-migration   DECISIONS.md @ 0.603 gained              -> multi-region
        fr-package-pricing       DECISIONS.md @ 0.600, DESIGN.md @ 0.600  -> weak-coverage
        fr-price-history-export  DECISIONS.md @ 0.603 lost                -> weak-coverage
        fr-supersession          design/01-foundation @ 0.600 lost        -> anchored:no-account

    The two **gains** are honest: the new entries genuinely discuss window coverage and version
    scheduling, so those requirements really do have a second account now.

    The three **losses are threshold-straddlers, not document defects**, and the distinction is
    load-bearing because only one of them changes the judged count. Every lost region scored within
    0.003 of `SCORE_THRESHOLD`, and in each case the losing document **was never edited by this
    wave** — `design/01-foundation.md`, `DESIGN.md`, and the pre-existing half of `DECISIONS.md`.
    The mechanism is the corpus-wide document-frequency cutoff: ~121 lines of new content changes
    which terms are ubiquitous enough to drop, which changes a requirement's discriminating-term
    set, which changes the *recall* of an unchanged window. It is visible directly in
    `fr-supersession`'s untouched id anchors, which moved by one thousandth in the same run
    (`DESIGN.md` 0.118 → 0.119, `design/01-foundation.md` 0.164 → 0.165). Its design account still
    exists in prose; the search stopped counting it as an account, which is a fact about the search
    and is exactly what `anchored:no-account` is for.

    Nothing was reworded to hold a score and no threshold was touched (rules 5 and 6). That the
    metric can be moved at the third decimal by unrelated corpus growth is recorded here as a
    property of `SCORE_THRESHOLD` being a hard cut — it belongs with SKILL.md's terse-prose note,
    and a docs wave is the wrong place to change a scorer, least of all on the sample that exposed
    it. Per-id record beside `PINNED_TRIAGE_PRICING` and `PINNED_JUDGE_CALLS`.

23. **2026-08-07, twenty-third capture — after the D-237…D-246 docs wave.** Document movement
    only, no checker change. Ten register entries from the Slice-4 owner round (the currency/tax
    strand's owed register plus D-238's two Critical alarms and D-246's gated-market gauge), and
    one correction to D-239 written *because of this run*.

    **Live findings unchanged at 2** (both rating-side, untouched). **Suppressed 59 → 58**: one
    pinned `P3/code-unreferenced` member paid down, `BRAND_UNKNOWN` / `design/04-currency-tax.md`,
    **and it is the first member in the pin's history removed by deleting the declaration rather
    than by naming the code in the rule that raises it.** D-239 splits the taxonomy refusal by
    *surface* rather than by class, so §5 no longer declares the per-class trio and
    `inst-tx-brand` answers `SCOPE_VALUE_UNKNOWN` for every taxonomy-backed class. That is the
    honest resolution of this finding's own claim — the debt said "declared, and nothing raises
    it", and the fix was to stop declaring it. Hand-checked: the token now appears nowhere in
    `gears/bss/pricing/docs` except the D-239 entry recording the strike, which declares nothing.

    **A prediction was made before the run and it was wrong, which is why this paragraph exists.**
    The predicted drop was **three** — the three codes D-239 strikes. The actual drop is **one**.
    `PARTNER_UNKNOWN` and `ORG_TIER_UNKNOWN` were declared in the same §5 block *and* named in
    `inst-tx-brand`'s prose, so P3 saw them referenced and they were never pinned. D-239's entry
    had asserted that "spec-check's `code-unreferenced` debt for the three is correct"; it was
    correct for one, and the entry is corrected in the register rather than quietly edited.
    Entry 15 above warns that a bare mention can *pay* a pin with no rule raising the code; this
    is its mirror — **a bare mention can also keep a code out of the debt it belongs in.** Same
    fact about P3 in both directions: it counts references, and a reference is not a rule.

    **The triage pin did not move at all** — `test_pricing_triage_histogram_is_pinned` runs
    `--gear gears/bss/pricing/docs` against the live corpus and passes unchanged (3 / 5 / 59 / 10,
    judged 69, total 77). After entries 12–22 each recorded movement, a ten-entry wave moving
    nothing is worth stating plainly rather than assuming a mistake.

    **No line number moved either, and that is a defect rather than a virtue — the finding of this
    capture.** Every previous multi-entry wave shifted all 21 `P1/propagation-missing` anchors
    uniformly, one line per status-board row it added. This one shifted none, because its whole
    80-line diff sits at `DECISIONS.md:2150`, below the deepest anchor (677). Chasing that
    anomaly found the cause: **the status board stopped being maintained at D-193.** It carries
    193 rows; the register carries 246 entries; **53 consecutive decisions, D-194 … D-246, have no
    board row**, and no board row lacks an entry — a clean stop, not corruption. It last grew in
    `9230dccc6`, the phase-4 close wave that is entry 22's subject. Nothing checks this: the board
    is not an invariant P1, P2 or P3 knows about, so it went stale for eight waves in silence.

    Deliberately **not fixed in this capture**, and the reason is mechanical rather than
    reluctant: 53 rows land above line 632, which would shift all 21 anchors and force a second
    re-capture in the same commit, conflating a pin payment with a bulk renumber. Two slice
    strands are also in flight and will hand back decisions of their own, so a backfill now is a
    backfill done twice. Recorded here as owed work with its cost stated.

### 24. 2026-08-09 — the clone route, and a pin that would have been paid falsely

**What moved:** one `P3/code-unreferenced` member, `CLONE_SOURCE_NOT_FOUND` /
`design/12-operator-efficiency.md`. Suppressed known debt 54 → 53; pinned code
list 33 → 32. Live findings unchanged at 2, both rating-side.

**Why it moved, and why that sentence needed checking.** D-277 built
`POST /plans/{planId}/clone`; D-278 minted the code the route answers when the
source holds no current revision. But the *first* thing that closed this finding
was neither: it was the **register entry mentioning the code**. P3 asks whether
any document references the code, and D-278's prose does, so the pin moved before
a single rule had been written. That is a false payment, and it is the second
time this program has walked into it — the first was caught by noticing the
baseline slide from 54 to 53 with no rule changed.

The real payment is `inst-cl-source`, a new clause 6 in §3's clone list, which
states the rule that raises the code: the source is the plan's **current**
revision, a plan holding only a draft has none and answers
`CLONE_SOURCE_NOT_FOUND` rather than a bare not-found, and a retired plan *does*
hold a current revision and is therefore clonable.

**How the difference was established rather than assumed:** the register mention
was temporarily rewritten to name the refusal in words, the checker re-run, and
the finding confirmed **still closed** — so the rule is the closer and the
register is not load-bearing. Restore-and-verify, not inspection.

### 25. 2026-08-09 — Phase 2, and the same trap a third time

**What moved:** one `P3/code-unreferenced` member, `BULK_ROW_CONFLICT` /
`design/12-operator-efficiency.md`. Suppressed known debt 53 → 52; pinned code
list 32 → 31. Live findings unchanged at 2.

**Why it moved.** D-291 built the bulk import's Phase 2, which raises the code —
a stale `ETag`, a row a neighbouring run holds, or a row whose assertion and the
draft plane disagree. But the *first* thing that closed the finding was the
register entry mentioning the code, exactly as in entry 24. **Third time.**

The real payment is `inst-bk-phase2`, which now names the code and enumerates the
three facts it covers.

**Verified the same way**: the register mention was reworded to prose, the checker
re-run, and the finding confirmed still closed. Restore-and-verify, not
inspection.

**On the recurrence.** Two entries now record this trap and it was sprung again,
which suggests the guard should not be a habit. The cheapest structural fix would
be a spec-check rule that ignores `DECISIONS.md` when deciding whether a declared
code is *referenced* — the register is where decisions about codes are recorded,
not where rules live. Noted here rather than built: it is a change to what the
checker means by "referenced", and that is the skill's contract.

24. **2026-08-14, twenty-fourth capture — D-312, and four waves that had gone
    unpinned behind it.** Document movement only, no checker change. **Live findings
    unchanged at 2** (the two cross-gear coverage statements). **Suppressed 52 → 49.**

    **The capture is two stories and they are deliberately not merged.** The oracles
    had not been re-captured since D-291, so D-307, D-309, D-310 and D-311 were all
    sitting unpinned; running the suite before touching anything already showed three
    reds. Everything below is attributed by **measurement against a worktree at
    `3492f0091`** — the commit before D-312's design edits — rather than by
    proximity, because a capture that credits one wave with another's movement is
    worse than a stale pin.

    - **D-312's own, two members:** `EVAL_POLICY_MISPLACED` / design/03, now named by
      `inst-mk-forbidden`, and `RESERVATION_ON_NON_USAGE` / design/10, now named by
      `inst-rv-attrs`. Both are the honest resolution — the rule that raises the code
      names it.
    - **Not D-312's, one member:** `RUN_SELECTOR_EMPTY` / design/12. D-312 never
      touched design/12; the code had already left the unreferenced set at the
      pre-edit commit.
    - **Not D-312's, the whole triage histogram.** `anchored:no-account` 7 → 4,
      `suspicious:multi-region` 57 → 61, `suspicious:weak-coverage` 10 → 9, and
      `PINNED_JUDGE_CALLS["pricing"]` 67 → 70. Computed at `3492f0091` and after
      D-312: **all seven buckets identical, 77 neighbourhoods on both sides.** The
      control is not ceremony — a wave that moves coverage and one that only moves
      document-frequency sampling are indistinguishable in this histogram.

    **The register-prose trap sprang a third time, and it was mine.** D-312's entry
    carries an inventory table whose left column is the bare token
    `EVAL_POLICY_MISPLACED`; that mention alone closed the `code-unreferenced`
    finding while `inst-mk-forbidden` still named nothing. It was caught by the
    worktree measurement above rather than by reading the entry — which is the whole
    argument for measuring. Resolved by naming the code in the rule and then verified
    the way entries 15 and 23 prescribe: the register mention was removed, the
    checker re-run, and the finding confirmed **still closed**. Restore-and-verify,
    not inspection.

    **Three occurrences is no longer a habit problem.** Entry 23 proposes the
    structural fix — a rule that ignores `DECISIONS.md` when deciding whether a
    declared code is *referenced*, the register being where decisions about codes are
    recorded and not where rules live. This capture is the third data point for it.
    Still not built here, for entry 23's reason: it changes what the checker means by
    "referenced", and that is the skill's contract, not a capture's business.

25. **2026-08-16, twenty-fifth capture — the checker moved, not the documents, and the
    live run goes red.** The **fourth** capture in this file's history justified by
    *checker* behaviour rather than document movement (entries 4 and the 2026-07-31
    resolver extension are the others), and the **first** in which the live run stops
    exiting 0. **Live findings 2 → 3. Suppressed unchanged at 49.** Not one existing
    finding moved: `live-text.txt` and `live-show-known-debt.txt` differ by exactly one
    added line and the count line, every pinned member reproduces at the same line
    number, and `live-json.json` gains one object.

    **Two defects were repaired, both of the "reports success on something it never
    checked" family.**

    - **A propagation target outside the shorthand table was dropped, and not reported.**
      `targets.resolve` knew six forms — `S<n>`, `Foundation`, `PRD`, `DESIGN`,
      `SEAMS <id>`, `ADR-NNNN` — plus the explicit cross-gear path. Anything else in a
      `**Propagated**:` field vanished, and `propagation-uninterpretable` did *not* fire,
      because that finding is guarded on the **whole** citation resolving to nothing and
      these citations always carried a shorthand that did resolve. The claim therefore
      read exactly like a verified one. `PRD` and `DESIGN` were never shorthands: they are
      the stems of two top-level documents that happened to be hard-coded, so the fix is
      to derive the vocabulary from the corpus — every top-level `*.md` contributes its
      stem, and any corpus-relative `*.md` path resolves as written. A path of that shape
      naming a document the corpus does not hold is now `propagation-unresolvable`
      instead of silence, reported per target rather than per citation.
    - **`**Propagated**:` was parsed per physical line.** A citation that wraps had only
      its first line resolved; every target below the wrap went unchecked and unreported.
      The field is now rebuilt as its markdown block, ended by a blank line, a list item
      (`- **Amended by …**`, including D-319's indented sub-bullet), a heading or a table
      row.

    **Three claims had never been checked in the life of this tool, and all three verify
    clean.** D-43 → `STRIPE-GAP-ANALYSIS.md` ("STRIPE-GAP-ANALYSIS G-2 marked actioned",
    the stem form) and D-319 → `STRIPE-GAP-ANALYSIS.md` (the path form; D-319's own entry
    predicted this and said so in prose) are cited 5 and 4 times in that document;
    SUB-D-19 → `REVIEW.md` ("REVIEW F-08-1 → fixed") is cited 3 times. That they are
    *checked* rather than merely quiet was established by measurement, not inspection:
    stripping the decision id from each target document produces the expected
    `propagation-missing` and nothing else, and the same probe against the pre-fix tool
    produces **nothing at all** for all three. That probe is pinned as
    `test_the_previously_unchecked_live_claims_are_now_armed_against_their_targets`.

    **One claim was wrong, and it is the whole of the finding-count move.**
    `D-313 -> PRD.md`, Medium, `DECISIONS.md:3461`. D-313's field wraps over four lines
    and its `PRD` token sits on line **two**, inside the clause "rating PRD §Definitions,
    §Time and §539" — a *cross-gear* claim written in prose. As written it names the
    citing gear's own `PRD.md`, which cites D-313 **0** times (measured). The resolvable
    form is `../../rating/docs/PRD.md`, which the resolver has understood since
    2026-07-31.

    **It was deliberately not pinned.** `PINNED_PROPAGATION_GAPS_2026_07_29` is a
    snapshot of *accepted* debt taken on one day, and the D-46 precedent recorded beside
    it is the rule — a brand-new finding put there is a finding buried. It lives in
    `LIVE_UNACCEPTED_GAPS_2026_08_16` in `test_propagation.py` instead, which is compared
    but never suppressed, so the finding stays in the CLI's output and **the run exits
    1**. `test_cli.py`'s `LIVE_EXIT_CODE` records that, and one further test asserts the
    exit code's *reason* rather than only its value. Fixing it is the register owner's
    call; this program does not edit gear documents to make its own checker green.

    **The regression was measured across every gear the tool loads, not assumed.** Before
    and after, `--gear <g>/docs --auto-context --show-known-debt` for all five BSS gears
    with a `docs/` tree: pricing **0 live / 49 debt → 1 / 49**, rating 2 → 2,
    subscriptions 0 → 0, ledger 24 → 24, products 1 → 1. Independently, `resolve` was run
    over all **275** parsed propagation claims in the three registers on both sides and
    diffed: **six** changed, **every one a pure addition** — D-313 (+`PRD.md`, the
    finding), D-314 (block now read to its end; its continuation names only
    `sqlite_window_service.rs` and a Rust module header, so no target is added), D-319
    and D-43 (+`STRIPE-GAP-ANALYSIS.md`), D-68 (+`design/03`, +`design/10`, both cited),
    SUB-D-19 (+`REVIEW.md`). **No claim lost a target, and nothing that was reported
    before stopped being reported.** D-324, the other candidate carried into this work,
    did **not** move: its four shorthand targets all resolved before and all four cite it.

    **The triage pin was not touched** — no neighbourhood or requirement code changed, and
    `test_pricing_triage_histogram_is_pinned` passes unchanged.

    Suite: **233 passed → 259 passed**, 1 skipped throughout; 26 added (9 wrapped-field,
    11 target-vocabulary, 5 P1-level, 1 gate-reason). Every one was run against the
    pre-fix tool in a scratch copy and against three deliberate mis-fixes — an over-greedy
    block rule, a stem vocabulary that swallows `PRD`/`DESIGN`/`SEAMS` and nested file
    stems, and a resolver that forgets to exclude already-claimed spans. A test that
    passes against the tool it is supposed to be pinning has proved nothing.

26. **2026-08-16, twenty-sixth entry — a checker fix. Nothing was re-captured, and that
    is the finding.** Entry 25's repair prescribed a remedy for `D-313 -> PRD.md`: write
    the cross-gear claim in the resolvable form. **The remedy could not be applied.**
    Written in the register's own house style —
    `[rating PRD](../../rating/docs/PRD.md)`, a form the register already contains
    verbatim elsewhere — the finding does not clear, because the link *label*'s `PRD` mints
    a second, phantom claim into the **citing** gear's own `PRD.md`, and the phantom is the
    one that fails. Prescribing a fix nobody can apply is its own defect, so it is repaired
    here rather than carried as follow-up.

    **Two rules, and the second is the one the register needed.**

    - **A shorthand is a citation token, not a piece of a longer word.** `\b` held on both
      sides of the `Foundation` in D-172's "the third-**Foundation**-refusal paragraph",
      because `-` is a non-word character, so an English compound minted
      `design/01-foundation.md`. Rejected now: a word character, a `-`, a leading `.`, and
      a trailing `.` followed by an alphanumeric (an extension). A trailing sentence period
      is still fine.
    - **A markdown link is one target.** When `[label](dest)`'s destination is a document
      target, the whole link is that target — the same doctrine that has governed a
      shorthand inside a bare path since 2026-07-31, extended to the form an author
      actually writes. A link whose destination is not a document claims nothing.

    **`/` is deliberately not rejected on either side of a shorthand, and that is measured.**
    `DESIGN/README` is a real live citation of `DESIGN.md` (D-03) and `S7/S11` is the same
    shape waiting to be written; rejecting `/` would silently drop both, which is the exact
    defect class entry 25 exists to repair. Path segments are excluded by *claiming the
    span* of the whole path, which is exact, not by guessing from punctuation, which is not.

    **The class was re-checked, not the instance.** Every live claim naming a
    `../../<gear>/docs/<FILE>.md` path whose `<FILE>` is also a shorthand: **two**, both
    D-66's — `../../rating/docs/DESIGN.md` and `../../subscriptions/docs/PRD.md`. Neither
    is a phantom today, because the span-claiming added 2026-07-31 already covers the bare
    and backticked path forms; had it not, both would have produced **false**
    `propagation-missing`, since pricing's own `DESIGN.md` and `PRD.md` cite D-66 **0**
    times. The **link** form was the uncovered case, and no live Propagated field used it
    yet — D-68's is the only link in any of them and its label is a path, so it was
    indistinguishable. Exactly **one** live phantom existed, D-172's `Foundation`, and it
    was **masked by duplication**: D-172 also cites `S1`, which resolves to the same
    `design/01-foundation.md`, so the phantom was absorbed into a real target and no output
    ever differed. Masked, not absent — which is why the class was measured rather than
    inferred from the finding count.

    **Nothing moved, measured across everything the tool loads.** `resolve` was run over
    all **275** parsed claims in the three registers at `4dd40ad2c` and after, and diffed:
    **zero changed**. All three oracle files are byte-identical and were left untouched;
    `--gear <g>/docs --auto-context --show-known-debt` is byte-identical for all five gears
    (pricing 1 live / 49 debt, rating 2, subscriptions 0, ledger 24, products 1); the
    pricing run still exits 1 on the same single finding. No pin moved.

    Suite: **259 -> 269 passed**, 1 skipped. Seven of the ten added tests fail against
    `4dd40ad2c` outright, including the end-to-end one that writes D-313's remedy into the
    live register in memory and requires the finding to clear. The other three are bounds,
    and each was made to discriminate: two crude mis-fixes (reject every path-ish character
    around a token; claim every markdown link span regardless of destination) and one
    half-fix (handle only cross-gear link destinations) each turn one of them red. Two of
    the three were rewritten when the first attempt passed against all of them — a bound
    test that no wrong implementation can fail is not a bound.

27. **2026-08-16, twenty-seventh capture — a document fix, and the exact opposite of
    entry 26.** Entries 25 and 26 were checker changes; this one changes **one clause of
    one register entry** and no code at all. **Live findings 3 -> 2. Suppressed unchanged
    at 49. The live run exits 0 again.**

    D-313's `**Propagated**:` field said `rating PRD §Definitions, §Time and §539` — a
    cross-gear claim written in prose, which as written named the **citing** gear's own
    `PRD.md`. It now reads `` `../../rating/docs/PRD.md` ``. That is the whole diff: three
    lines re-wrapped, ten characters of prose replaced by a path, nothing else in D-313
    and no other entry.

    **The form was chosen by measurement, not by instruction.** Four candidate forms were
    written into the live register in memory and all four clear the finding (the bare
    backticked path; a link labelled with the full path; a link labelled with the
    basename; a link with a prose label — the last only since entry 26). The control, the
    unchanged text, still fails, so the probe was armed. The form applied is **D-66's**,
    because D-66 is the **only precedent inside a `**Propagated**:` field** and it writes
    bare backticked paths; the eleven markdown links in this register all sit in prose
    bodies, nine of them labelled with a basename and none with its own full destination
    path. A link whose label repeats its destination is a shape the register uses nowhere.

    **The claim is verified, not merely quiet, and that was measured both ways.** Rating's
    `PRD.md` cites D-313 **three** times; stripping those three produces exactly
    `D-313 claims propagation into ../../rating/docs/PRD.md, but that document never cites
    D-313` and nothing else. Run pricing **alone** and the claim now reports
    `P1/propagation-target-not-loaded` — honest about needing the sibling gear, the same
    answer D-66's targets give, and strictly better than the old text's silent pass
    against a same-named own-gear document.

    **What moved, by name.** `--gear <g>/docs --auto-context --show-known-debt`: pricing
    **1 live / 49 debt, exit 1 -> 0 live / 49 debt, exit 0**; rating, subscriptions,
    ledger and products all **byte-identical**. The three-gear run 3 -> 2 and exit 1 -> 0.
    In `live-show-known-debt.txt` exactly one line is removed and every one of the 21
    pinned `P1/propagation-missing` members reproduces **at the same line number** — the
    edit keeps D-313's field at four lines, and every pinned anchor is at
    `DECISIONS.md:677` or above anyway. Suppressed unchanged; no member paid down, none
    added; the triage pin untouched.

    **All three oracle files are now byte-identical to their pre-entry-25 selves**, which
    is worth stating plainly rather than leaving to be noticed: the checker got better,
    surfaced a real gap, the gap was fixed, and the output returned to what it had always
    printed. The two live findings are the same two — but they are now the only two there
    *are*, rather than the only two the tool could see.

    **Pins re-taken from measurement.** `LIVE_UNACCEPTED_GAPS_2026_08_16` loses its one
    member and is **kept as an empty tuple** with the closure recorded beside it: it is
    the slot a newly surfaced, unaccepted gap belongs in, and an existing empty slot is
    what stops the next one being dropped into the accepted-debt pin for want of anywhere
    else to put it. `LIVE_EXIT_CODE` 1 -> 0. The gate test was rewritten from "fails on
    exactly this finding" to "passes because nothing live is above the gate, and every
    live finding is Low" — the shape that would catch a Medium being buried in the pin
    rather than fixed, so it outlives the finding it was written for.

    **`test_the_prescribed_fix_for_d_313_actually_clears_its_finding` had its premise
    removed and was not deleted.** It patched a broken citation in memory and required the
    finding to clear; the citation is no longer broken. It became
    `test_d_313_cross_gear_claim_stays_resolvable_and_is_actually_checked`, which asserts
    three things in the order they can fail — the citation resolves to the sibling
    document **and to no in-corpus `PRD.md`**, it verifies clean today, and it is
    *checked*, proved by stripping the id from rating's PRD. The third leg is the point:
    without it the test would pass against a checker that dropped the target entirely,
    which is the defect class this whole branch exists to guard. A second test pins the
    pricing-alone answer. Both were proved red against the pre-edit register, and both
    against a checker mutated to drop cross-gear targets — the pre-2026-07-31 behaviour —
    which the "no finding" assertion alone could never have caught.

    Suite: **269 -> 270 passed**, 1 skipped (one test replaced by two).

28. **2026-08-16, twenty-eighth capture — the D-330 historical-import descope.** Document
    movement only, no checker change. **Live findings unchanged at 2** (the two rating-side
    statements). **Suppressed 47 -> 45.** `live-text.txt` and `live-json.json` differ from their
    predecessors in the debt count and **nothing else**; `live-show-known-debt.txt` loses exactly
    two lines, and eleven surviving members shift **+1** from the one line D-13's strike record
    added above them.

    D-330 takes historical import **out of scope** and strikes the flow rather than deferring it:
    Slice 5 §2's seven steps and Slice 11 §3's synthesis consumer leave the design set —
    **eight declared instructions**, measured `grep -oE '\`inst-[a-z0-9-]+\`$'` per slice: S5
    **37 -> 30**, S11 **38 -> 37**. Tier 1 of synthesis is untouched and D-87's consumability
    argument survives; only its premise that the payload's source is an imported store goes.

    **Both debt movers are named, and they are different kinds of payment.**

    - **`BACKDATE_GRANT_REQUIRED` / design/05** left `PINNED_UNREFERENCED_CODES_2026_07_29` because
      its **declaration was deleted** — the *second* member ever paid that way, after D-239's
      `BRAND_UNKNOWN`, and the same shape: the debt said "declared, and nothing raises it", and the
      fix was to stop declaring it. `BACKDATE_SIDE_EFFECT` was struck in the same edit and was
      **never a member**, for D-239's recorded reason exactly — it was declared in the block *and*
      named by the rule body, by D-81 and by two ACs, so P3 always saw it referenced. Both are now
      named **outside** the Problem-responses block, in prose that declares nothing.
    - **`D-13 -> PRD.md`** left `PINNED_PROPAGATION_GAPS_2026_07_29` (21 -> 20) because the PRD's
      strike record for `fr-historical-import-governance` names D-13 as one of the rules that went
      with the flow. Stated precisely rather than glossed: the gap was paid by the PRD finally
      *citing* the decision, which is what P1 asks — and the requirement D-13 propagated into is
      the one that was struck, so the citation is the record and not a technicality.

    **The requirement pin moved down for the first time.** `fr-historical-import-governance`'s
    `**ID**:` declaration is **removed**, so pricing goes **fr 66 -> 65, total 78 -> 77**, and
    Slice 5's `**Traces to**:` line drops it in the same edit. Annotating the declaration in place
    would have left P2 reporting a requirement no slice owns — a struck requirement is not an
    unclaimed one. A denominator here is deliberately not monotone, which is D-239's precedent for
    error codes and D-330 clause (1) for instructions.

    **P1 was armed rather than assumed.** The `D-330` token was stripped from each of its seven
    resolvable targets in turn, in a scratch copy: **exactly one** `propagation-missing` appeared
    each time, naming exactly the stripped document, with the rest of the finding set constant —
    `design/05`, `design/11`, `DESIGN.md`, `PRD.md`, `design/01`, `design/02`, `design/README.md`.

    **The triage pin moved, and the two movers are not one story.** Total 78 -> 77,
    `multi-region` 63 -> 62, `anchored:no-account` 4 -> 3, `weak-coverage` 8 -> 9;
    `PINNED_JUDGE_CALLS["pricing"]` **unchanged at 71**.
    `fr-historical-import-governance` leaves the corpus, which is the whole of the total's and
    `multi-region`'s -1, and applying **only** the PRD edit to the pre-wave tree reproduces that
    and nothing else. `fr-priceoverlay-referential-integrity` crossed `anchored:no-account` ->
    `weak-coverage` on the **sixth** crossing of one boundary by the same route: it gained
    `DECISIONS.md:19-30`, the register's status paragraph, scoring **0.625** against
    `SCORE_THRESHOLD = 0.6` on 5 matched terms, over a region that is **byte-identical** across the
    wave. **No single edited document reproduces it** — each of the eight was applied to the
    pre-wave tree alone and all eight leave the id where it was, as do the PRD alone and
    `DECISIONS.md` alone — so it is document-frequency movement over a fixed window grid and not
    evidence about coverage. The wave touched neither that requirement nor
    `design/09-price-overlays.md`.

    **One thing the strike forced, worth carrying because the next struck instruction will meet
    it.** A struck `inst-*` id can never be written in backticks again: a backticked id is a
    *reference* to P3, and a reference to an id no bullet declares is `inst-dangling`, **Medium**.
    Leaving the eight ids backticked in the register and in the two 2026-07-30/31 review records
    produced **five** such findings — measured, then resolved by naming them as plain text
    wherever the record has to name them. It is the mirror of entry 15's warning: a bare token can
    *pay* a pin falsely, and a backticked one can *manufacture* a finding out of a document that
    is merely remembering.

    Suite: **270 passed**, 1 skipped — unchanged.

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
