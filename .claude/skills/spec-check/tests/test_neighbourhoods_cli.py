import json
import subprocess
import sys

from conftest import REPO_ROOT, SCRIPTS

NEIGHBOURHOODS_PY = SCRIPTS / "neighbourhoods.py"


def run(*args):
    proc = subprocess.run(
        [sys.executable, str(NEIGHBOURHOODS_PY)] + list(args),
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    return proc


def test_anchored_no_account_is_not_judged_by_default(tmp_path):
    # The ladder's budget guarantee: promoting this bucket takes ledger from 17
    # judged to 40, so it must never happen by accident.
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/ledger/docs", "--out", str(out)).returncode == 0
    hoods = json.loads(out.read_text(encoding="utf-8"))["neighbourhoods"]
    anchored = [n for n in hoods if n["triage"] == "anchored:no-account"]
    assert anchored and not any(n["judge"] for n in anchored)


def test_judge_anchored_promotes_the_bucket(tmp_path):
    # Converting "the search found no account" into an answer someone can act on.
    out = tmp_path / "n.json"
    proc = run("--gear", "gears/bss/ledger/docs", "--out", str(out), "--judge-anchored")
    assert proc.returncode == 0, proc.stderr
    hoods = json.loads(out.read_text(encoding="utf-8"))["neighbourhoods"]
    anchored = [n for n in hoods if n["triage"] == "anchored:no-account"]
    assert anchored and all(n["judge"] for n in anchored)
    judged = sum(1 for n in hoods if n["judge"])
    assert judged == 40


def test_the_histogram_does_not_lie_about_what_it_will_cost(tmp_path):
    # The printed line is what a reader budgets from. Under the flag it must say
    # the bucket is judged and count it into the dispatch total.
    def line(stdout, label):
        # Padding depends on the longest class name, so compare on content.
        for row in stdout.splitlines():
            if row.startswith(label):
                return " ".join(row.split())
        raise AssertionError("no {!r} line in:\n{}".format(label, stdout))

    out = tmp_path / "n.json"
    default = run("--gear", "gears/bss/ledger/docs", "--out", str(out))
    promoted = run("--gear", "gears/bss/ledger/docs", "--out", str(out), "--judge-anchored")
    assert line(default.stdout, "anchored:no-account") == "anchored:no-account 23 (not judged)"
    assert line(promoted.stdout, "anchored:no-account") == "anchored:no-account 23 (judged)"
    assert line(default.stdout, "judge calls") == "judge calls 17"
    assert line(promoted.stdout, "judge calls") == "judge calls 40"


def test_writes_an_envelope_and_prints_a_histogram(tmp_path):
    out = tmp_path / "n.json"
    proc = run("--gear", "gears/bss/ledger/docs", "--out", str(out))
    assert proc.returncode == 0, proc.stderr
    assert proc.stderr == ""
    envelope = json.loads(out.read_text(encoding="utf-8"))
    assert envelope["gears"] == ["gears/bss/ledger/docs"]
    assert envelope["thresholds"]["window_lines"] == 12
    assert envelope["thresholds"]["window_step"] == 6
    assert envelope["thresholds"]["score_threshold"] == 0.6
    assert envelope["thresholds"]["strong_score"] == 0.75
    assert envelope["thresholds"]["document_frequency_cutoff"] == 0.25
    assert envelope["thresholds"]["max_fragments"] == 6
    assert len(envelope["neighbourhoods"]) == 40
    # Every class named in the histogram, including the zero ones: a class that
    # silently stops occurring must not look like a class that never existed.
    for line in ("unbuildable:no-prose", "no-region", "covered:strong"):
        assert line in proc.stdout


def test_every_requirement_gets_exactly_one_neighbourhood(tmp_path):
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--out", str(out)).returncode == 0
    envelope = json.loads(out.read_text(encoding="utf-8"))
    ids = [n["requirement_id"] for n in envelope["neighbourhoods"]]
    assert len(ids) == 77
    assert len(set(ids)) == 77
    assert all(n["triage"] for n in envelope["neighbourhoods"])


def test_no_neighbourhood_exceeds_the_fragment_budget(tmp_path):
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--out", str(out)).returncode == 0
    envelope = json.loads(out.read_text(encoding="utf-8"))
    for item in envelope["neighbourhoods"]:
        assert len(item["fragments"]) <= envelope["thresholds"]["max_fragments"]


def test_output_is_byte_stable_across_runs(tmp_path):
    first, second = tmp_path / "a.json", tmp_path / "b.json"
    run("--gear", "gears/bss/ledger/docs", "--out", str(first))
    run("--gear", "gears/bss/ledger/docs", "--out", str(second))
    assert first.read_bytes() == second.read_bytes()


def test_only_id_selects_a_single_requirement(tmp_path):
    out = tmp_path / "n.json"
    assert run(
        "--gear", "gears/bss/ledger/docs",
        "--only-id", "cpt-cf-bss-ledger-fr-idempotency-per-flow",
        "--out", str(out),
    ).returncode == 0
    envelope = json.loads(out.read_text(encoding="utf-8"))
    assert [n["requirement_id"] for n in envelope["neighbourhoods"]] == [
        "cpt-cf-bss-ledger-fr-idempotency-per-flow"
    ]


def test_only_id_file_selects_a_sample(tmp_path):
    ids = tmp_path / "ids.txt"
    ids.write_text(
        "# the sample\n"
        "cpt-cf-bss-ledger-fr-idempotency-per-flow\n"
        "\n"
        "cpt-cf-bss-ledger-fr-money-rounding-scale\n",
        encoding="utf-8",
    )
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/ledger/docs", "--only-id-file", str(ids),
               "--out", str(out)).returncode == 0
    envelope = json.loads(out.read_text(encoding="utf-8"))
    assert sorted(n["requirement_id"] for n in envelope["neighbourhoods"]) == [
        "cpt-cf-bss-ledger-fr-idempotency-per-flow",
        "cpt-cf-bss-ledger-fr-money-rounding-scale",
    ]


def test_an_unknown_requested_id_is_an_error_not_a_silently_smaller_sample(tmp_path):
    # An evaluation run that judges 15 of 16 and says nothing is the exact failure
    # this whole tool exists to catch.
    proc = run("--gear", "gears/bss/ledger/docs", "--only-id", "cpt-cf-bss-ledger-fr-nope",
               "--out", str(tmp_path / "n.json"))
    assert proc.returncode == 1
    assert proc.stderr.startswith("Error: ")
    assert "declared by no loaded gear" in proc.stderr


def test_only_class_filters(tmp_path):
    out = tmp_path / "n.json"
    assert run(
        "--gear", "gears/bss/ledger/docs", "--only-class", "covered:strong", "--out", str(out),
    ).returncode == 0
    envelope = json.loads(out.read_text(encoding="utf-8"))
    assert {n["triage"] for n in envelope["neighbourhoods"]} <= {"covered:strong"}


def test_a_missing_docs_tree_is_an_error_not_an_empty_run(tmp_path):
    proc = run("--gear", "gears/bss/nope/docs", "--out", str(tmp_path / "n.json"))
    assert proc.returncode == 1
    assert proc.stderr.startswith("Error: ")
    assert not (tmp_path / "n.json").exists()


def test_an_unknown_only_class_is_a_usage_error(tmp_path):
    proc = run(
        "--gear", "gears/bss/ledger/docs", "--only-class", "not-a-class",
        "--out", str(tmp_path / "n.json"),
    )
    assert proc.returncode == 2


#: Verified by hand 2026-07-30 before being frozen, at score threshold 0.6 and
#: strong 0.75, with multiplicity counted over *accounts* (regions clearing the
#: threshold) rather than over citations.
#:
#: Not the first output of the code, twice over. The first run used the design's
#: absolute term counts and put 116 of 116 requirements in
#: `suspicious:multi-region`, making three of six classes unreachable. The second
#: counted any id anchor towards multiplicity and sent 110 of 116 to a judge — the
#: cost the ladder exists to avoid — because an anchor carrying one term of 53 was
#: treated as an account of the rule.
#:
#: What was checked by eye:
#:
#: - `no-region` × 6, all pricing NFRs (read latency, event propagation,
#:   multi-currency scale, mass-repricing throughput, availability/DR, size limits).
#:   Checked negatively: the design slices mention `p95 < 100ms` in ten places, every
#:   one a reference to a budget the PRD defines, and no slice specifies an SLO.
#: - `anchored:no-account` × 18 + 23. `fr-addon-rules` is the shape: one anchored
#:   region carrying **one term of 53** (score 0.019). The id is named; the rule is
#:   not there. Ten of the sixteen requirements P2 reports as claimed by 2–5 ledger
#:   slices land here, which is a substantive result on its own.
#: - `suspicious:multi-region`: ledger's `fr-idempotency-per-flow` is anchored in
#:   four distinct slices scoring 0.000, 0.571, 0.143 and 0.286 — only one is an
#:   account, so under the corrected rule it is *not* multi-region.
#: - **Overlapping windows were inflating this class by a factor of two.** Reading a
#:   real batch prompt showed `fr-manual-adjustment-governance` carrying
#:   `design/05:415-426` and `design/05:409-420` as two regions — the same seven
#:   governance steps, shown to the judge twice. Deduplicating overlapping windows
#:   took `multi-region` from 34 to 14 in pricing and 9 to 3 in ledger: 20 of
#:   pricing's 34 "two accounts" were one paragraph counted twice, which is a
#:   false-divergence source removed, not merely budget saved.
#: - `covered:strong` × 0 and `not-normative` × 0 are honest zeroes: the first needs
#:   a single account that is *also* id-anchored and ≥ 0.75, and requirement prose in
#:   both corpora is overwhelmingly `MUST`-laden. Both stay in the histogram at zero
#:   so a class that stops occurring is distinguishable from one that never existed.
#: **Moved back on 2026-08-08 by the D-256 composite-meter wave, to exactly the
#: pre-wave values (5 / 59 / 10, judged 69).** `fr-scheduled-migration` and
#: `fr-supersession` returned to `multi-region` and `weak-coverage`: prose added to
#: `design/10` re-weighted `DF_CUTOFF` the other way and pushed their 0.603 / 0.602
#: accounts back over the 0.600 floor. This is the same mechanism the note above
#: records, observed in both directions within one day -- which is the strongest
#: evidence available that these pins track corpus term statistics rather than
#: coverage, and that pricing has requirements whose whole account lives inside a
#: 0.003 band. Nothing about either requirement's documentation changed in either
#: direction.
#: **Third movement in one day, back to 7 / 58 / 9** (D-259's tail wave). The two
#: knife-edge requirements have now crossed the 0.600 floor three times in a
#: single session, on three waves none of which edited either requirement or its
#: accounts. Re-pinned rather than tuned, for the reason the first note gives.
PINNED_TRIAGE_PRICING = {
    "unbuildable:no-prose": 0,
    "no-region": 3,
    "anchored:no-account": 4,
    "suspicious:multi-region": 62,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 8,
    "covered:strong": 0,
}
#: Moved 2026-08-14, later the same day (was 61 / 9), by **D-312's bulk-import arm
#: prose** — and it is the marginal-score artefact this file has now recorded four
#: times, not a coverage change. One mover, hand-checked:
#:   - `fr-price-history-export`, `weak-coverage` -> `multi-region`. It gained a
#:     `DECISIONS.md` fragment scoring **0.603** against `SCORE_THRESHOLD = 0.6`.
#:     Its genuine account is `design/12-operator-efficiency.md` at 0.707; the new
#:     fragment is D-312's account of closing the bulk-import door, which shares the
#:     export requirement's operator/batch/row/catalog vocabulary while being about
#:     key contradictions and not about export at all.
#: Attributed by measurement, not proximity: a worktree at 18c6a18f7 — the commit
#: before the D-312 bulk arm's entry text — ran the whole suite **233 passed, 1
#: skipped**, and `check.py --auto-context` printed `0 finding(s)` / 49 suppressed on
#: both sides. So the deterministic layer is byte-identical and only this histogram
#: moved. Not re-worded to push the score back under the threshold: the prose is
#: correct documentation and reshaping a document to move a metric is worse than
#: recording the artefact. The cost of leaving it is one spurious judged
#: neighbourhood — an N1 run will be asked whether two accounts of
#: `fr-price-history-export` diverge, and one of them is about bulk import.
#: Moved 2026-08-14 (was 7 / 57 / 10) and **not by D-312**, which is the point of
#: this note. The oracles had not been re-captured since D-291, so four decision
#: waves (D-307, D-309, D-310, D-311) were sitting unpinned and surfaced together
#: with the D-312 capture. Attributed by measurement rather than by proximity: the
#: histogram was computed against a worktree at 3492f0091 — the commit before
#: D-312's design edits — and every one of the seven buckets was identical to the
#: post-D-312 run, 77 neighbourhoods on both sides. The control matters here
#: because a docs wave that *does* move coverage and one that only moves
#: document-frequency sampling are indistinguishable in this histogram.
#: Moved 2026-08-10 (was 58 / 9) by **D-305's register entry**, and the move is a
#: sampling artefact rather than a coverage change — worth stating because the two
#: are indistinguishable in the histogram. `fr-bundle-composition` went
#: `multi-region` -> `weak-coverage` by losing its `DECISIONS.md` fragment, while
#: the register still carries ten mentions of that requirement: the entry appended
#: at the end of the file pushed the sampled fragment past `MAX_FRAGMENTS`. So a
#: bucket move means either "the evidence changed" or "an unrelated append
#: displaced a sample", and only diffing the per-requirement triage against the
#: previous revision tells them apart. That diff is how this one was identified.
#: Moved 2026-08-08 (was 5 / 59 / 10) by the **D-251/D-252/D-234 propagation wave** — the
#: `EQUAL_PRECEDENCE_CROSS_CLASS_TIE` catalogue entry in `design/09` §5, the D-251 and D-252
#: accounts in `design/11` §3 and §5, and four in-line corrections in `DECISIONS.md`. Two movers,
#: **both into `anchored:no-account` and both by the document-frequency mechanism this file has
#: now recorded three times**, not by any account being weakened:
#:   - `fr-scheduled-migration`, `multi-region` -> `no-account`. Its two accounts —
#:     `DECISIONS.md:25-36` and `design/11-lifecycle.md:193-204` — both scored **0.603** against
#:     `SCORE_THRESHOLD = 0.6` and fell under it together. Neither window was edited by this wave.
#:   - `fr-supersession`, `weak-coverage` -> `no-account`. Its single account,
#:     `design/01-foundation.md:541-552`, scored **0.602**. Its document was not edited either,
#:     and this is the *second* time this requirement has moved on this mechanism (see the
#:     2026-08-05 note beside `PINNED_JUDGE_CALLS`, where the same account scored exactly 0.600).
#: The control is that every surviving `id-anchor` score in both neighbourhoods rose by ~0.002 in
#: the same run (0.141 -> 0.143, 0.157 -> 0.159, 0.111 -> 0.112): `DF_CUTOFF` is computed per
#: corpus and never curated, so adding windows anywhere re-weights every term, and three regions
#: sitting 0.002-0.003 above a hard floor changed side. **What this keeps showing is a property of
#: the corpus, not of the wave**: pricing has requirements whose entire account rests within
#: 0.003 of the selection threshold, so any docs wave in any of the three gears can silently
#: reclassify them. Recorded rather than tuned — moving the threshold to hold these two would
#: re-open the distribution knee the constant was measured at.
#: Moved 2026-08-06 (was 4 / 10 above, i.e. `anchored:no-account` 4 and `weak-coverage` 11) by the
#: **Slice 9 overlay merge docs wave** — D-219…D-233 in `DECISIONS.md`, plus their citations in
#: S9 §3/§5/§6 and S8 `inst-ba-material`. Total requirements unchanged at 77; the judged count
#: moves 70 -> 69 because this is a **judged-to-unjudged** move, unlike the D-196 wave's.
#:
#:   id                  region that moved                                direction
#:   fr-event-contract   DESIGN.md:85-96 @ 0.478 and                      -> anchored:no-account
#:                       design/01-foundation.md:73-84 @ 0.609
#:
#: **The control here is stronger than a geometry argument, because the geometry did not move at
#: all.** The requirement keeps the *same three candidate regions*, at the same files and the same
#: line ranges, and their text is **byte-identical** on both sides — checked by regenerating the
#: pre-wave corpus with `git archive HEAD` and diffing the emitted fragments rather than by
#: reading. What moved is corpus-relative and nothing else: `matched_terms` 11 -> 10 and 14 -> 13,
#: scores 0.478 -> 0.455 and 0.609 -> 0.591, both crossing the account threshold from just above
#: to just below. Across the whole corpus **no region was gained or lost** and exactly two shared
#: regions changed text — the two paragraphs this wave edited in S9, neither of them this
#: requirement's.
#:
#: So this is the `catalog`/`rule` document-frequency class recorded further down, not ladder
#: drift and not a document defect: ~15 register entries of new prose moved the document frequency
#: of a term this requirement's vocabulary depends on, and two marginal matches fell 0.02 under a
#: threshold they were sitting 0.02 over. The evidence for `fr-event-contract` is the same evidence
#: it had before the wave; what changed is how a corpus-relative score weighs it.
#: Moved 2026-08-06 (was 3 / 4 / 60 / 10) by the **D-196 decision wave** — the owner's answer to
#: D-196 written into `DECISIONS.md`, S1 §3.7, S1 §4.1 and S2 `inst-cs-usage`. Total requirements
#: unchanged at 77 and the judged count unchanged at 70: the move is +1/−1 **inside** the judged
#: set, both classes being in `JUDGED`.
#:
#: **One mover, and no control was needed to classify it** — the grid could not have moved under
#: it, which is a cheaper argument than the blank-line control and a stronger one:
#:
#:   id                    region that moved                              direction
#:   nfr-audit-retention   design/01-foundation §3.7 @ 0.656 lost         -> weak-coverage
#:
#: The lost window is the `pricing_price` schema bullet at line 541. Both of this wave's edits to
#: that document are elsewhere — the §4.1 axis statement is an **insertion at line 598**, below it,
#: and §3.7's own edit rewrote one sentence **inside a single line** without changing the line
#: count. Windows are 12 lines stepping 6 from the top of the file, so every window boundary at or
#: above line 541 is byte-identical in position: the score fell because that window's *text*
#: changed, not because the grid re-sliced under it. That is the distinction entries 18-21 needed
#: the control to make, and here the geometry settles it.
#:
#: **And the drop is honest.** The window scored 0.656 against a 0.6 threshold — a marginal match
#: that survived on term profile rather than on substance: it accounted for retention nowhere,
#: carrying `created_by` "(the history-export actor field)" and the append-only trigger, neither of
#: which states a retention horizon. The D-196 rewrite added ~120 words of index mechanics (NULL
#: distinctness, the `COALESCE` sentinel) to the same window, diluting a similarity that was thin
#: to begin with. Nothing audit-related was removed: the audit tokens present in the pre-wave line
#: — `created_by`, `REVOKE`, "Append-only", "history-export" — are all present in the post-wave
#: one, checked by diffing the tokens on both sides of the replacement rather than by reading it.
#: The two surviving fragments are the NFR sentence itself and the audit-record step list, which
#: are the two regions that actually account for it.
#:
#: Moved 2026-08-05 (was 3 / 5 / 59 / 10, judged 69) by the **D-183…D-193 phase-4 close docs wave**
#: — eleven register entries plus a status-board paragraph and one clause each in S3, S5 and S7.
#: Total requirements unchanged at 77; **judged 69 -> 68**, the single mover out of the judged set
#: being `fr-supersession`.
#:
#: **Five triage movers, all hand-checked per id against the pre-wave tree (`b65201b52`) in a
#: detached worktree, and the blank-line control reproduces NONE of them** — 121 blank lines in
#: `DECISIONS.md` and 2 each in S5/S7, no content anywhere, leaves the histogram byte-identical to
#: pre-wave. So every mover here is content, not the window grid re-slicing, which is the opposite
#: of entries 18-21's finding and is why the control was worth running:
#:
#:   id                       region that moved                        direction
#:   fr-future-gap-coverage   DECISIONS.md @ 0.625 gained              -> multi-region
#:   fr-scheduled-migration   DECISIONS.md @ 0.603 gained              -> multi-region
#:   fr-package-pricing       DECISIONS.md @ 0.600, DESIGN.md @ 0.600  -> weak-coverage
#:   fr-price-history-export  DECISIONS.md @ 0.603 lost                -> weak-coverage
#:   fr-supersession          design/01-foundation @ 0.600 lost        -> anchored:no-account
#:
#: Two kinds, and they need opposite readings. The two **gains** are honest content: the new
#: entries genuinely discuss window coverage (D-184/D-189/D-190/D-191) and version scheduling
#: (D-185/D-188/D-192), so those requirements really do have a second account now.
#:
#: The three **losses** are threshold-straddlers and **not** document defects. Every lost region
#: scored within 0.003 of `SCORE_THRESHOLD` (0.600, 0.600, 0.603), and in each case the losing
#: document was **never edited by this wave** — `design/01-foundation.md`, `DESIGN.md` and the
#: pre-existing half of `DECISIONS.md`. What moved is the corpus-wide document-frequency cutoff:
#: adding ~121 lines of content changes which terms are ubiquitous enough to drop, which changes a
#: requirement's discriminating-term set, which changes the *recall* of an unchanged window. The
#: shift is visible directly in `fr-supersession`'s untouched id anchors, which moved by exactly one
#: thousandth in the same run (`DESIGN.md` 0.118 -> 0.119, `design/01-foundation.md` 0.164 ->
#: 0.165). `fr-supersession`'s design account still exists in prose; the search stopped counting it
#: as an account.
#:
#: Recorded rather than tuned. The metric being sensitive to corpus growth at the third decimal is
#: a property of `SCORE_THRESHOLD` as a hard cut, already acknowledged in SKILL.md's terse-prose
#: note, and a docs wave is not the place to change a scorer — least of all on the sample that
#: exposed it (standing rule 6).
#: **Held unmoved 2026-08-04** through the **D-182 docs wave** — the window plane's single register
#: entry, plus one status-board row, one preamble sentence and one clause each in S7
#: `inst-fg-trailing` and S11 `inst-rt-cancel`. Recorded although nothing moved, because the note
#: below named the register preamble a permanent threshold-straddler and this is the first wave to
#: append to it since: a zero from that window is a measurement, not an absence of one.
#:
#: **Zero triage movers, zero judge movers, three region movers**, each diffed per id against the
#: pre-wave tree (`4d007405`) in a detached worktree, and the blank-line control (1 / 9 blank lines
#: at this wave's two `DECISIONS.md` insertion points, no content anywhere) separates them exactly:
#:
#:   id                        window before -> after                      control
#:   fr-prepaid-credit-grant   DECISIONS.md:547-558 -> 553-564             reproduced
#:   nfr-size-limits           DECISIONS.md:1357-1368 -> region dropped    reproduced
#:   fr-future-gap-coverage    design/07:205-216 -> 199-210                NOT reproduced
#:
#: The first two are the grid re-slicing pre-existing text on a +10-line offset, with **zero**
#: content contribution, and neither changes class (4 -> 4 regions; and 3 -> 2 stays
#: `multi-region`, two regions still over threshold). The third is this wave's content and is the
#: one that should have moved: the wave grows `inst-fg-trailing` **in place**, both the losing and
#: the winning window contain that rule's line, the region count stays at 2 and the class stays
#: `weak-coverage` — the window re-centres on the rule that grew. `design/07` gains no lines, which
#: is why the control cannot reproduce it.
#:
#: **The straddling window was measured directly rather than argued about.** `DECISIONS.md:19-30`
#: grew 66,743 -> 68,440 bytes and its region membership is **44 ids before, 44 after, 44 in the
#: control — the same 44, none gained, none lost.** So the note below is not weakened: a window
#: carrying a large fraction of every requirement's vocabulary is also one an ordinary append
#: cannot push further. No term-level `DF_CUTOFF` sweep was run, deliberately — with zero triage
#: movers there is no crossing to attribute, and computing one would invite reading a number as a
#: cause. Nothing was reworded (rule 6); no threshold was touched.
#:
#: (Previous move, recorded below.) Moved 2026-08-03 (was 4 / 5 / 54 / 14, judged 68, total unchanged at 77) after the
#: **phase-closing docs wave** — the wave that lands D-180 (`submit`/`withdraw` join S5 §6's
#: `action` roster), D-181 (the correlation id is minted, not adopted from an inbound trace id)
#: and two id-less records of what the phase proved by execution.
#:
#: **Six movers, and not one is a coverage change.** Each was diffed per id against the pre-wave
#: tree (`e7704c10`) in a detached worktree, and each gains exactly **one** new `term-overlap`
#: region scoring **0.600–0.667** — every one of them just over the 0.6 threshold, and every one
#: of them in one of only **two** windows:
#:
#:   id                          window                          score before -> after
#:   fr-bundle-composition       DECISIONS.md:19-30              0.595 -> 0.603
#:   fr-customer-group-pricing   DECISIONS.md:19-30              0.593 -> 0.613
#:   fr-customer-group-pricing   design/01-foundation.md:541-552 0.593 -> 0.600
#:   fr-model-kind               DECISIONS.md:19-30              0.590 -> 0.667
#:   fr-sellability-gate         DECISIONS.md:19-30              0.574 -> 0.609
#:   nfr-event-propagation       design/01-foundation.md:541-552 0.588 -> 0.647
#:   nfr-publish-propagation     design/01-foundation.md:541-552 0.588 -> 0.647
#:
#: **The newly matched terms decide it.** Term-for-term, what crossed each requirement into its
#: new class is generic vocabulary this wave's prose happens to contain, never a statement about
#: the requirement: `primary` (bundle composition), `groups`/`many` (customer-group pricing),
#: `applies`/`implicit`/`matrix` (model kind), `applies`/`execution`/`offered`/`void`
#: (sellability gate), `events` (event propagation), `rating` (publish propagation) — the last
#: two from one clause naming the four gears the outbox emits to. The wave is about the audit
#: `action` vocabulary and the correlation id; it says nothing about bundles, customer groups,
#: model kinds or sellability, and neither new window cites any of the six ids.
#:
#: **The mechanism is the two mega-windows, and it is worth naming as measurement health.**
#: `DECISIONS.md:19-30` is the register's TOC plus its **single-line, 67 KB** wave preamble, and
#: `design/01-foundation.md:541-552` is §3.7's bullet block at **29 KB**. A region's score is the
#: *fraction* of a requirement's discriminating terms the window carries, so a 12-line window
#: that happens to hold one enormous paragraph carries a large fraction of **any** vocabulary and
#: sits permanently within a term or two of 0.6. Four of the six moved on 1 term of 116, 3 of 39,
#: 4 of 115 and 1 of 17 respectively; at 17 terms the score's own granularity (0.059) is coarser
#: than the distance these ids sat from the threshold. Any wave that appends to either place will
#: flip a handful of ids in whichever direction it happens to push. That is a property of the
#: instrument meeting two documents that grew a paragraph-per-line habit, not of the design set.
#:
#: **One `DF_CUTOFF` crossing, and it is the known oscillator.** Exhaustively over the 5,141
#: terms both trees share, exactly one crossed: **`unit` 0.24426 -> 0.25070**, moving *outside*
#: and so ceasing to discriminate — the same term entry 20 recorded crossing *inside*
#: (0.25042 -> 0.24661) one wave earlier, oscillating on window-count dilution alone (2,481 ->
#: 2,501 windows). It touches exactly one mover and there the arithmetic is complete without any
#: content at all: `fr-customer-group-pricing` on `design/01-foundation.md:541-552` matches **48**
#: terms before and after, and 48/81 = 0.593 becomes 48/80 = 0.600 — the threshold crossed by a
#: shrinking denominator and nothing else. `catalog` 0.23700 -> 0.23471 and `same` 0.25796 ->
#: 0.26190 moved as expected and did not cross.
#:
#: **The blank-line control is decisive in the other direction, and that is new.** The pre-wave
#: tree with **26 / 19 / 72 blank lines** at this wave's three insertion points and no content at
#: all reproduces the *old* histogram exactly (4 / 5 / 54 / 14). So unlike entries 18–20 this
#: move is **not** the grid re-slicing pre-existing text: the line count is innocent and the added
#: words are the cause. What the words are is the paragraph above — six ids each tipped by one to
#: four terms of ordinary English inside a window that was already 96–99% of the way there.
#: Nothing was reworded to move a score back (rule 6) and no threshold was touched.
#:
#: (Previous move, recorded below.) Moved 2026-08-03 (was 4 / 6 / 54 / 13, judged 67, total unchanged at 77) after the
#: **D-179 docs wave** — phase 3's group **G1**, which mints the publish-path code for the
#: freeze D-177 described and repairs the `deny`-token contradiction in S5 §6.
#:
#: **One mover, and it is the window grid alone.** `fr-supersession`
#: `anchored:no-account` -> `suspicious:weak-coverage`, gaining a single `term-overlap` region
#: in `design/01-foundation.md` at **0.604** — four thousandths over the 0.6 threshold. Its
#: three id-anchored regions are unchanged at 0.072 / 0.117 / 0.162.
#:
#: **This exactly reverses the previous capture's departure, and it is the same oscillator.**
#: Entry 19 recorded this id falling 0.6091 -> 0.5676 when 38 lines went in above §3.7, and
#: flagged it as threshold-adjacent for two consecutive waves. This wave adds **13** lines to
#: `design/01-foundation.md` §3.3 — the `PRIMITIVE_RULES_UNBUILT` declaration — and the id
#: comes back over.
#:
#: **One controlled run, and it is decisive.** Pre-wave tree (`c7c2f872`) with **13 blank
#: lines** inserted at the exact insertion point of this wave's §3.3 paragraph and **no content
#: at all**: identical counts, and `fr-supersession` at the identical **0.604**. The prose
#: contributes nothing — the new paragraph is about two unauthorable Slice-10 primitives and
#: does not mention supersession — so the region is pre-existing §3.7 text that a 12-line window
#: at step 6 re-slices back onto. The grid is necessary and sufficient; there is no `DF_CUTOFF`
#: component to decompose because there is no content component either.
#:
#: **What this adds to the model.** Entries 18 and 19 established the grid mechanism on
#: *downward* moves and reproduced them under blank lines. This is the first capture to
#: reproduce an **upward** move the same way, on the same id, in the opposite direction, from an
#: edit thirteen lines long. The measurement-health signal entry 19 recorded — a 12-line window
#: at step 6 is a coarse instrument near 0.6 — now has a matched pair. Nothing was reworded to
#: move the score in either direction (rule 6), and no threshold was touched.
#:
#: (Previous move, recorded below.) Moved 2026-08-03 (was 3 / 5 / 52 / 17, judged 69, total unchanged at 77) after the
#: **D-175…D-178 docs wave** — the seventh implementation-side wave, raised by Group **G8**,
#: whose purpose was closing six waves of the register's own owed-back clauses rather than
#: building a plane. **Four movers**, each diffed per-id against the pre-wave tree (`5a8c801e`)
#: in a detached worktree, with **five** controlled runs. **This is the first move in eight
#: captures that LOWERS the judged share (69 -> 67), and the whole of the -2 is the window
#: grid.**
#:
#: **One `DF_CUTOFF` component, the first in four edits — and it is not the cause of anything.**
#: Checked exhaustively over the 4,992 terms both trees share: exactly one crossed
#: `DF_CUTOFF = 0.25`, **`unit` 0.25042 -> 0.24661**, moving *inside* and therefore becoming
#: discriminating. The mechanism is dilution rather than usage: the pricing corpus went from
#: 2,404 to 2,433 windows (this wave adds lines, `WINDOW_STEP` is 6), so every term's document
#: frequency fell slightly and `unit` was sitting four ten-thousandths outside. It touches one
#: mover, `fr-supersession`, by adding **1** to a 110-term denominator, and the arithmetic below
#: shows it is neither necessary nor sufficient there. The two historical oscillators moved as
#: expected and did not cross: `catalog` 0.24626 -> 0.24127 (further inside), `same`
#: 0.25707 -> 0.26141 (further outside).
#:
#: **Five controlled runs split the four movers exactly 2 + 2.**
#: (i) Pre-wave tree with **only** `DESIGN.md` and `DECISIONS.md` replaced: the two
#:     `mentions`-class gainers below and nothing else.
#: (ii) Pre-wave tree with everything **except** those two replaced (the PRD and the five
#:     design slices this wave propagates into): the two departures and nothing else.
#: (iii) Pre-wave tree with **38 blank lines** inserted at the two points this wave adds text
#:     to `design/01-foundation.md` (7 above §3.3's precondition block, 31 inside it) — the
#:     exact shift this wave puts above §3.7, with **no content at all**: **both** departures
#:     reproduce exactly. The grid is sufficient on its own.
#: (iv) Pre-wave tree with **only** `design/01-foundation.md` replaced: the same two
#:     departures, so no other edited file contributes to them.
#: (v) Pre-wave tree with the PRD, S2, S5, S10 and S12 replaced and S1 left alone: **zero**
#:     movers. Five edited design surfaces and a PRD edit move nothing whatever.
#:
#: - `fr-per-seat` weak -> multi. Gains `DECISIONS.md:19-30`, the register preamble — the
#:   documented `mentions` class, and the same window that promoted this id one wave ago. Judge-
#:   neutral (both classes are in `JUDGED`).
#: - `fr-price-history-export` weak -> multi. Gains `DECISIONS.md:19-30` as well, and nothing
#:   else. Also judge-neutral. Both of these are the preamble growing by a wave narrative that
#:   summarises every requirement in the gear, which is why any requirement's vocabulary can
#:   match it.
#: - `nfr-event-propagation` weak -> **no-region**, and it is a pure grid effect with **no** DF
#:   component (`unit` is not among its 17 discriminating terms). Its only region was
#:   `design/01-foundation.md:487-498` at 11/17 = **0.6471**; post-wave the best window that
#:   §3.7's bullets fall into scores 9/17 = **0.5294**, under the 0.6 threshold, so the id has no
#:   region at all. Nothing about its neighbourhood changed in substance — `pricing_outbox`'s
#:   bullet is still its genuine account and this wave *extended* that bullet (D-178's
#:   correlation producer) — but 38 lines were added above §3.7, the 12-line window at step 6
#:   re-slices, and two of the eleven matched terms now fall on opposite sides of a boundary.
#:   **This exactly reverses the previous capture's promotion**, which entry 18 recorded as
#:   arriving on the single preposition `within` from a rejected-alternative clause.
#: - `fr-supersession` weak -> **anchored:no-account**. 67/110 = **0.6091** at
#:   `design/01-foundation.md:487-498` — a hundredth above threshold, which entry 18 flagged as
#:   threshold-adjacent for two consecutive waves and reproduced under blank lines alone — to
#:   63/111 = **0.5676** post-wave. The decomposition matters because `unit` is in this id's
#:   terms: at the **old** denominator the new match count still fails (63/110 = 0.5727 < 0.6),
#:   and at the **new** denominator the old match count still passes (67/111 = 0.6036 > 0.6). So
#:   the grid is necessary and sufficient and the DF crossing is neither; it contributes 0.005 of
#:   a 0.041 fall. Its other regions are id-anchored only, hence `anchored:no-account`.
#:
#: **Judged 69 -> 67 = 87%, ending seven consecutive waves of growth — and the mechanism is the
#: measurement, not the documents.** Said plainly, as the wave brief asks: this wave's increment
#: comes from **neither** a design slice **nor** the digest/preamble in any load-bearing sense.
#: The two `mentions` gains are the obligatory summary documents as always and are judge-neutral;
#: the entire judged-share change is two threshold-adjacent ids losing a window they only ever
#: held by a hundredth, because text was added *above* the window they held. Entry 18 predicted
#: exactly this for both ids and named the mechanism (a blank-line shift reproduces it), so this
#: capture is that prediction coming true rather than a new phenomenon. Nothing was reworded in
#: either direction to chase the number, and no threshold was touched: what a re-slice this
#: fragile actually says is that a 12-line window at step 6 is a coarse instrument near 0.6, and
#: that is recorded as a measurement-health signal for whoever tunes it.
#:
#: (Previous move, recorded below.) Moved 2026-08-03, **sixth move of the day** (was 4 / 5 / 49 / 19, judged 68, total unchanged
#: at 77) after the **D-170…D-174 docs wave** — the sixth implementation-side wave, raised by
#: building Group **G7**, the gear's REST surface, and the one that closes Phase 2. **Four
#: movers**, each diffed per-id against the pre-wave tree (`f8f3ed51`) in a detached worktree,
#: with four controlled runs.
#:
#: **No `DF_CUTOFF` component, for the third edit running.** Checked exhaustively over the
#: 4,923 terms both trees share: **zero** crossed `DF_CUTOFF = 0.25` in either direction. The
#: two terms that have oscillated around it in previous captures both moved further inside —
#: `catalog` 0.24685 -> 0.24626, `same` 0.25105 -> 0.25707. So all four movers are region gains.
#:
#: **Four controlled runs split them exactly, and the split is 3 + 1.**
#: (i) Pre-wave tree with **only** `DESIGN.md` and `DECISIONS.md` replaced: the first three
#:     movers below and nothing else.
#: (ii) Pre-wave tree with everything **except** those two documents replaced — the five
#:     register entries' propagation edits in three design slices and the PRD: the fourth
#:     mover and nothing else.
#: (iii) Post-wave tree with the D-174 sentence removed **in place** from the
#:     `pricing_idempotency_dedup` bullet (one enormous line, so every line number stays
#:     identical): the fourth mover reverts. It is content, not the window grid.
#: (iv) Pre-wave tree with **26 blank lines** inserted at §3.3 — the exact shift this wave puts
#:     above §3.7, and no content at all: **zero** of the four movers, so the grid contributes
#:     nothing to any of them. It has one effect of its own, `fr-supersession` weak ->
#:     no-account, which the wave's real content cancels; that requirement has now been within
#:     a hundredth of the threshold on the same §3.7 window for two consecutive waves, on terms
#:     (`batching`/`delay` last wave, this wave's §3.7 additions now) that say nothing about
#:     supersession.
#:
#: - `fr-approval-threshold-policy` weak -> multi. Gains `DESIGN.md:499-510`, 82/138 = 0.594 ->
#:   86/138 = 0.623. Four terms, all from the D-72 digest's new paragraph: **`direction`** and
#:   **`routes`** (the overlay stack sort direction; "the nine authoring routes"), **`multi`**
#:   (D-173's multi-facet update) and **`regardless`**. Nothing about approval thresholds.
#: - `fr-priceoverlay-authoring` weak -> multi. Gains `DESIGN.md:505-516`, 147/248 = 0.593 ->
#:   153/248 = 0.617, and `DECISIONS.md:19-30`, 141/248 = 0.569 -> 149/248 = 0.601. Terms
#:   **`stack`**, **`discards`**, **`layer`**, **`strictly`**, **`tariffs`**. This is the one
#:   mover of the four whose new region says anything about its own subject: both windows carry
#:   the wave's standing-list correction, which is *about* the overlay stack (D-138 closed the
#:   `fixed` arithmetic; the sort direction survives as its own fork, and matters more because a
#:   `fixed` line discards every layer beneath it). It is a **status fact about a fork**, not a
#:   statement of the authoring rule, so a judge should still be expected to answer `mentions`.
#: - `fr-tenant-brand-isolation` weak -> multi. Gains `DECISIONS.md:19-30`, 13/22 = 0.591 ->
#:   15/22 = 0.682, and `DESIGN.md:499-510`, 13/22 = 0.591 -> 14/22 = 0.636. A **22-term**
#:   denominator, so one term is worth 0.045 — the smallest declaration in either window's
#:   reach. The terms are **`authz`** (from "their authz gate", describing what G7 built) and
#:   **`mutating`** (from "every mutating route"). Neither window says anything about tenant or
#:   brand isolation.
#: - `nfr-event-propagation` no-region -> weak. Gains `design/01-foundation.md:487-498` at
#:   11/17 = 0.647, up from 10/17 = 0.588 on the **same window with the same line numbers**
#:   (run (iii)). **One term: `within`**, and it comes from D-174's *rejected-alternative*
#:   clause quoting §3.3 — "additive-only **within** a major version" — against the
#:   requirement's own "MUST reach downstream consumers **within** 5 seconds at p95". The
#:   window's other ten matched terms (`carry`, `consumers`, `correlation`, `downstream`,
#:   `idempotency`, `keys`, `least`, `once`, `other`, `safe`) are `pricing_outbox`'s bullet —
#:   frozen event names, dedup/correlation keys, `(tenantId, aggregateId)` ordering — which is
#:   genuinely this requirement's neighbourhood, is **unchanged by this wave**, and was
#:   sub-threshold before it. D-174 created no account; it tipped an old one over 0.6 on a
#:   preposition. Nothing was reworded: `within` is the right word for the sentence it is in.
#:
#: **Judged 68 -> 69 = 90%, the seventh consecutive wave to grow the share — and the first
#: whose increment is not the digest or the preamble.** The three weak -> multi moves are
#: judge-neutral (both classes are in `JUDGED`), so the whole of the +1 is
#: `nfr-event-propagation` entering out of `no-region`, from a **design slice**. That is a
#: change in the shape of the signal rather than in its direction, and it is worth stating
#: plainly: the cause is still not an account — it is one common preposition in a clause
#: explaining an option that was rejected — but for the first time the vehicle is a normative
#: slice rather than the two documents a wave is obliged to grow. Recorded as a
#: measurement-health signal, not tuned away.
#:
#: (Previous move, same day, recorded below.) Moved 2026-08-03, **fifth move of the day** (was 4 / 6 / 46 / 21, judged 67, total unchanged
#: at 77) after the **D-169 register edit** — the product owner's answer to the §F.1 fork D-168
#: opened the same day: `crossBoundaryWarningText` leaves the Slice-6 consumer contract, the
#: catalog publishes the K3 marker alone, and the surface that renders the warning owns its
#: copy. **Four movers**, each diffed per-id against the pre-edit tree (`8fb20548`) in a
#: detached worktree, with three controlled runs.
#:
#: **No `DF_CUTOFF` component, for the second edit running.** Checked exhaustively over the
#: 4,914 terms both trees share: **zero** crossed `DF_CUTOFF = 0.25` in either direction.
#: `same` — the term behind the fourth move of the day — went 0.25126 -> 0.25105 and stayed
#: out; `catalog` went 0.24537 -> 0.24685 and stayed in. So all four movers are region gains.
#:
#: **Each gained exactly one region, each region is the D-72 digest or the register preamble,
#: and each crossed `SCORE_THRESHOLD = 0.6` on exactly one term.** Every *other* region of
#: every mover is byte-identical across the two trees — same window, same score, same
#: matched-term count — so nothing that was already an account moved at all:
#: - `fr-per-seat` no-account -> weak. `DESIGN.md:499-510`, 15/26 -> 16/26 = 0.6154. The term
#:   is **`wording`**, from "the surface that renders the warning owns its wording".
#: - `fr-one-time-setup` weak -> multi. `DESIGN.md:499-510`, 19/32 -> 20/32 = 0.6250. The term
#:   is **`preview`**, from "the preview/migration UI" — the surface PRD AC #66 names.
#: - `fr-reserved-capacity` weak -> multi. `DESIGN.md:505-516`, 34/58 -> 35/58 = 0.6034. The
#:   term is **`authorable`**, from "makes the copy re-authorable going forward only".
#: - `fr-plan-clone` weak -> multi. `DECISIONS.md:19-30`, 25/43 -> 26/43 = 0.6047. The term is
#:   **`going`**, from that same clause about the rejected option (b), in the preamble.
#:
#: Not one of the four terms says anything about per-seat pricing, one-time setup fees,
#: reserved capacity or plan cloning, and two of the four come from one clause of one rejected
#: option. This is the `mentions` class at its purest yet: the eleventh entry's
#: `fr-tax-display-basis` crossed on the word `whole` and the sixteenth capture's
#: `fr-supersession` on `batching` + `delay`; here **four** requirements cross on one word
#: each, in a single edit, and three of them on one 12-line window.
#:
#: Three controlled runs separate it exactly. (i) Pre-edit tree + **only** the `DESIGN.md`
#: D-72 digest sentence: the three `DESIGN.md` movers and nothing else. (ii) Pre-edit tree +
#: **only** the `DECISIONS.md` preamble line: `fr-plan-clone` and nothing else. (iii) The
#: post-edit tree with the digest and the preamble reverted — so the D-169 entry, the
#: status-board row, the struck §F.1 row, the five Slice-6 edits and the two PRD edits are all
#: present: **zero** movers, histogram identical to the pre-edit pin. The decision's whole
#: substance moves the pin by nothing; the two documents D-72 and the wave convention *oblige*
#: an edit to grow move it by four.
#:
#: **Judged is now 68/77 = 88%, the sixth consecutive wave to grow it with no design slice
#: creating an account for any mover.** Recorded rather than tuned away: nothing was reworded
#: in either direction, and `wording`, `preview`, `authorable` and `going` are the right words
#: for the sentences they are in.
#:
#: (Previous move, same day, recorded below.) Moved 2026-08-03, **fourth move of the day** (was 4 / 9 / 44 / 20, judged 64, total
#: unchanged at 77) after the **D-163…D-168 docs wave** — the fifth implementation-side wave,
#: raised by building Group **G6**, the read side. **Six movers**, every one diffed per-id
#: against the pre-wave tree (`99523f15`) in a detached worktree, and four controlled runs
#: separate them exactly.
#:
#: - **One is the `DF_CUTOFF = 0.25` artifact**, and for the first time the term is not
#:   `catalog`: **`same`** moved 0.24968 -> 0.25126 and stopped being discriminating. It is the
#:   **only** term that crossed in either direction (checked exhaustively over the 4,835 terms
#:   both trees share; `catalog` moved 0.24797 -> 0.24537, further inside). Neutralising that
#:   one term on both trees removes `fr-one-time-setup` (multi -> weak, having lost the
#:   `DESIGN.md` 499-510 digest window at 0.606) and reproduces every other mover unchanged.
#: - **Three gained register prose and nothing else** — the `mentions` class the tenth capture
#:   named: `fr-min-qty-floor` (weak -> multi, `DESIGN.md` 499-510 at 0.605),
#:   `nfr-mutation-latency` (weak -> multi, the same window at 0.714) and `nfr-observability`
#:   (no-account -> weak, the `DECISIONS.md` preamble 19-30 at 0.633). Every new region of all
#:   three is either the D-72 register digest or the preamble — the two documents a wave is
#:   *obliged* to grow. A judge should be expected to answer `mentions` for all three.
#: - **One gained a genuine account, and it is the one requirement this wave is about.**
#:   `fr-publish-fanout-atomicity` (no-account -> multi) gains `DECISIONS.md:1399-1410` at 0.743
#:   — **D-166's entry**, which is what gives its degraded clause a start instant — plus the
#:   preamble at 0.786 and the digest at 0.657. Its two pre-existing regions **fell** in the
#:   same move (0.273 -> 0.186, 0.545 -> 0.400), the documented terse-prose recall effect: its
#:   PRD declaration grew by a paragraph, so the discriminating-term set grew and the unchanged
#:   anchors match a smaller fraction of it.
#: - **One is a pre-existing account promoted by two terms of adjacent growth, and it is worth
#:   the whole note.** `fr-supersession` (no-account -> weak) gains `design/01-foundation.md`
#:   463-474 — §3.7's table-bullet list — at **exactly 0.600**, up from 0.582 on the *same*
#:   window content: 64/110 discriminating terms matched, now 66/110. The two are **`batching`**
#:   and **`delay`**, both from D-166's vocabulary landing in the `pricing_catalog_version_ref`
#:   and `pricing_pin_frontier` bullets; neither has anything to do with supersession. What the
#:   window does carry about supersession — `supersedes_price_id`, the published-plane partial
#:   `UNIQUE`, the `published -> superseded` whitelist, the price-history bullet — is unchanged
#:   and was always there, always sub-threshold. Isolated by the cleanest controlled run
#:   available here: reverting **only** those two bullets to their pre-wave text *in place*
#:   (each bullet is one line, so every line number stays identical to the post tree) gives
#:   **zero** movers, so it is content and not the window grid; and the pre-wave tree plus
#:   **only** the six new status-board rows also gives **zero** movers, so no part of this move
#:   is the grid.
#:
#: **Not one mover gained a region in a design slice as an account of itself** — the closest is
#: `fr-supersession`, whose design-slice region is an old account tipped over by two words of
#: someone else's rule. This is the **fifth** consecutive wave to grow the judged share
#: (64 -> 67 = 87%) with no design slice creating an account for any mover, and it is recorded
#: as a measurement-health signal rather than tuned away: nothing was reworded in either
#: direction, and the two words that moved `fr-supersession` are the correct words for the
#: sentence they are in.
#:
#: Superseded record — the **third move of the day** (was 5 / 10 / 40 / 22, judged 62, total
#: unchanged at 77) after the **D-162 docs edit** — the product owner's answer to the §F.1 fork
#: D-161 opened the wave before: `pricingSnapshotRef`'s evaluation-policy segment is a
#: **vocabulary generation**, `ep-<n>`, over a nine-field roster the design set had never
#: written down, with an append-only log the gear's build replays so the bump cannot be
#: forgotten. **Six movers**, every one diffed per-id against the pre-edit tree (`e3fcd727`)
#: in a detached worktree.
#:
#: **This is the first move in four waves with no `DF_CUTOFF` component at all.** Checked
#: exhaustively over the union of both trees' terms (4,809 -> 4,835): **zero** terms crossed
#: `DF_CUTOFF = 0.25` in either direction. `catalog`, the term behind the twelfth entry and
#: the thirteenth and fourteenth captures, moved 0.24957 -> 0.24797 — further inside, not
#: across. So every mover here is a region gain, and all six have the same cause.
#:
#: **All six gained the two documents a wave is obliged to grow, and nothing else.** Every
#: new region of every mover is either `DESIGN.md` §4 (the D-72 register digest — windows
#: 499-510 and 505-516) or the `DECISIONS.md` "How to use this document" preamble (19-30).
#: **Not one gained a region in a design slice, in the PRD, or in the new normative §4.4
#: block this edit added** — which is worth stating precisely because §4.4 is where the whole
#: substance went.
#: - `nfr-read-latency` no-region -> weak. `DESIGN.md:499-510` at 0.643, its **first
#:   candidate region ever**. The digest paragraph mentions the read path; the requirement is
#:   a p95 latency number. A judge should answer `mentions`.
#: - `fr-bundle-composition` no-account -> weak. `DESIGN.md:505-516` at 0.607.
#: - `fr-custom-frequency` weak -> multi. `DESIGN.md:499-510` at 0.621.
#: - `fr-package-pricing` weak -> multi. `DESIGN.md:499-510` at **exactly 0.600**.
#: - `fr-invoice-currency-binding` weak -> multi. `DECISIONS.md:19-30` at **exactly 0.600**.
#:   Two of six landing on the threshold to the third decimal is the honest measure of how
#:   much of this move is signal.
#: - `fr-consumer-readmodel-resolution` weak -> multi, and the only one with a claim to being
#:   a real account: its declaration reads "model kind, ordered tier bands, and
#:   evaluation-policy fields", and the D-162 digest paragraph is the first text in the set
#:   that says which fields those are. It gains `DESIGN.md:505-516` at 0.638 and
#:   `DECISIONS.md:19-30` at 0.606; its `DECISIONS.md` body region re-anchored 919-930 ->
#:   913-924 at the **identical 0.617**, a window-grid effect of the one board row, not a
#:   second account.
#:
#: Controlled in both directions against the pre-edit tree. (i) Base + **only** the new
#: status-board row, nothing else: **zero** movers — so none of this is the grid. (ii) Base
#: with **only** `DESIGN.md` and `DECISIONS.md` replaced by their post-edit versions, every
#: design slice and the PRD left at `e3fcd727`: reproduces all six movers, in the same
#: directions, and nothing else. The design-slice and PRD edits of this wave move the pin by
#: **nothing**.
#:
#: **Judged is now 64/77 = 83%, and this is the fourth consecutive wave to grow it for
#: reasons that are not the design set.** Standing rule 4 says a class that stops being a
#: minority is a statement about the search rather than about the documents; the controlled
#: run above is the cleanest evidence yet that it is — a mandatory register digest, which
#: every requirement's vocabulary can match because it summarises all of them, is being
#: counted as a second account 44 times. Noted here and beside `PINNED_JUDGE_CALLS` rather
#: than acted on: changing the scorer is not a docs wave's business, and nothing was reworded
#: to move a score in either direction.
#:
#: (Previous move, same day, recorded below.) Moved 2026-08-03, **second move of the day** (was 5 / 14 / 28 / 30, judged 58, total
#: unchanged at 77) after the **D-155…D-161 docs wave** — the fourth implementation-side
#: wave, raised by building Group **G5**, the **publish commit** (the pipeline re-run inside
#: the commit transaction, the lifecycle flips, the transactional outbox, the fail-closed
#: `CatalogVersion` request, the segmented audit chain). This is the largest move any wave
#: has produced, and almost none of it is a design slice. **Fourteen movers**, every one
#: diffed per-id against the pre-wave tree (`a26991b8`) in a detached worktree; three causes,
#: separated by a controlled run.
#:
#: **(a) Three are the `catalog` document-frequency artifact** — the same single term as the
#: twelfth entry and the thirteenth capture, crossing `DF_CUTOFF = 0.25` for the third time
#: and in the admitting direction: 573/2290 = 0.25022 (rejected) -> 580/2324 = 0.24957
#: (admitted). Its own window count **rose**; the corpus grew faster. Checked exhaustively
#: over all 4,809 terms of the new tree against all 4,716 of the old: it is the **only** term
#: that crossed in either direction.
#: - `fr-catalogversion-increment` weak -> multi and `fr-event-contract` no-account -> weak
#:   gained **no new region at all**. Their existing regions merely crossed
#:   `SCORE_THRESHOLD = 0.6`: 0.579 / 0.579 / 0.632 -> 0.600 / 0.600 / 0.650, and
#:   0.455 / 0.591 -> 0.478 / 0.609. A pure threshold crossing on a term neither requirement's
#:   prose contains more or less of than before.
#: - `fr-model-kind-conformance` weak -> multi gained two regions at exactly 0.611 for the
#:   same reason (more of its terms became discriminating, so more windows scored).
#: Controlled run: dropping `catalog` from every requirement's term set on **both** trees
#: removes all three movers and reproduces nothing else. (It also surfaces one mover the live
#: pin does not have — `fr-custom-frequency` weak -> multi — which is the same artifact with
#: its sign flipped: the term's admission was *masking* a genuine gain there. That id does not
#: move in the real run and is not counted among the fourteen.)
#:
#: **(b) Two accounts the wave actually wrote** — and both are requirements it is about:
#: - `nfr-size-limits` weak -> multi. Gains `DECISIONS.md:1345-1356` at 0.657 — **D-160's
#:   entry**, which gives the ratified soft caps the advisory code they had never had, without
#:   which PRD §7.1's `SHOULD` had nothing to be reported through — plus the wave preamble at
#:   0.643. Its own PRD declaration was rewritten by the same decision. This id gained its
#:   first candidate region ever only one wave ago (thirteenth capture); it now has three.
#: - `fr-pricing-snapshot` no-account -> multi. Gains `DESIGN.md:505-516` at 0.771,
#:   `DECISIONS.md:19-30` at 0.747 and `DECISIONS.md:1345-1356` at 0.699 — **D-161's entry**,
#:   which is the account of what the catalog-side stamp contains and of the segment that has
#:   no producer. Its two id-anchored regions fell 0.538 -> 0.253 in the same move: the
#:   documented terse-prose recall effect, running the usual way — the declaration grew from
#:   one sentence to a paragraph, so its discriminating-term set grew while the unchanged
#:   anchors match a smaller fraction of it. The promotion is real regardless; the demotion of
#:   the anchors is the scorer.
#:
#: **(c) Nine gained register prose and nothing else** — the `mentions` class the tenth
#: capture named, all at 0.600-0.646: `fr-addon-rules`, `fr-billing-timing`,
#: `fr-customer-group-pricing`, `fr-grandfathering-eligibility`, `fr-level-aggregation`,
#: `fr-migration-safety`, `fr-plan-change-contract`, `fr-plan-retirement`,
#: `fr-trailing-tier-qualification`. **Every** new region of **all nine** is either
#: `DESIGN.md` §4 (the D-72 register digest — windows 499-510 and 505-516) or the
#: `DECISIONS.md` "How to use this document" preamble (windows 19-30 and 25-36). Not one of
#: the nine gained a region in a design slice, so no slice edit of this wave created an
#: account for any of them. What grew are precisely the two documents a wave is *obliged* to
#: grow: D-72 requires the DESIGN digest to stay current, and the preamble records each wave.
#: A judge should be expected to answer `mentions` for all nine, and the honest reading of the
#: +12 in `multi-region` is that most of it is the register getting longer, not the design set
#: getting better covered. Nothing was reworded to move a score in either direction.
#:
#: (Previous move, same day, recorded below.) Moved 2026-08-03 (was 6 / 14 / 21 / 36, judged 57, total unchanged at 77) after the
#: **D-149…D-154 docs wave** — the third implementation-side wave, raised by building
#: Group **G4**, the *shape of a plan* (Slice 2's four validator sets and its three
#: revision-scoped child tables), against the documents. Unlike the three edits before it
#: this wave really did write a lot of prose, so a large move is expected; what needed
#: checking is which part of it is the documents and which is the scorer. **Fourteen
#: movers**, every one diffed per-id against the pre-wave tree (`d5e18846`) in a detached
#: worktree. They split cleanly into three causes, and a controlled experiment separates
#: them exactly.
#:
#: **(a) Four accounts the wave actually wrote** — the requirements it is about:
#: - `fr-billing-cycles` weak → multi. Its own declaration was rewritten (D-149's two new
#:   codes), so its discriminating-term set grew 15 → 36, and it gains `DECISIONS.md:1249`
#:   (**D-149's entry**, 0.722) and `DECISIONS.md:19` (the wave preamble, 0.667) while
#:   `DESIGN.md:505` rises 0.600 → 0.750. D-149 *is* an account of the billing-cycle matrix.
#: - `fr-billing-descriptors` weak → multi (terms 46 → 87). Gains `design/02:247` — the
#:   descriptor algorithm, both of whose steps this wave rewrote (D-152, D-154) — at 0.724,
#:   plus the preamble and the S1 §3.7 `pricing_price` bullet D-154 extended.
#: - `fr-plan-phases` weak → multi (terms 113 → 131). Its `design/02` account moved 229 →
#:   235 with the window grid and stayed ~0.80; the second region is the `DESIGN.md:505`
#:   digest at 0.603.
#: - `fr-plantier-mandatory` weak → multi. Its composition account moved 211 → 217 and rose
#:   0.730 → 0.757 — this wave added `PLANTIER_DIVERGENT` to `inst-cmp-plantier`, paying
#:   down that pinned unreferenced code — and `design/01:403` clears at 0.649.
#:
#: **(b) Five that gained the D-72 register digest and nothing else** —
#: `fr-historical-import-governance` (0.611), `fr-included-allowance` (0.619),
#: `fr-mass-repricing` (0.610), `fr-one-time-setup` (0.606), `fr-proration-input-contract`
#: (0.600 exactly), plus `nfr-size-limits` **no-region → weak** (0.636, and the reason
#: `no-region` fell 6 → 5). Every one of these regions is `DESIGN.md:499`/`505` or
#: `DECISIONS.md:25` — the §4 register summary line and the register preamble, both of
#: which this wave lengthened substantially — and every score is inside 0.04 of the 0.6
#: threshold. This is the class the tenth capture already named: **register-digest prose
#: rather than rule statements**, on which a judge should be expected to answer `mentions`.
#: `nfr-size-limits` is the one with substance behind it as well — D-152 decides where its
#: "tenant-configurable" caps are declared, and the §4 sentence names them — and it went
#: from having **no candidate region at all** to having one.
#:
#: **(c) Four that are the `DF_CUTOFF = 0.25` artifact and nothing else.** Exactly **one**
#: term crossed the cutoff, and it is `catalog` again — the same term the D-145-amendment
#: note below records crossing in the *other* direction, which is how close to the line it
#: has been sitting for three waves. It was at 568/2272 = **0.25000**, discriminating only
#: because the test is `<=`; the corpus grew 2272 → 2290 windows and `catalog` gained five
#: of them, so 573/2290 = **0.25022** and it left the term set of every requirement that
#: had it. No other term crossed, in either direction. The four:
#: - `fr-catalogversion-increment` multi → weak. **No region gained or lost**; all three
#:   scores fall by the one term (11/19 = 0.579 twice, 12/19 = 0.632), so two of them stop
#:   clearing 0.6 and stop counting as accounts.
#: - `fr-event-contract` weak → anchored. Same shape, 14/23 = 0.609 → 13/22 = 0.591 on
#:   `design/01:73`, taking its last account under the threshold.
#: - `fr-model-kind-conformance` multi → weak. Loses two windows that had been at exactly
#:   11/18 = 0.611 and are 10/17 = 0.588 after.
#: - `fr-custom-frequency` anchored → weak, the same artifact **in the gaining direction**:
#:   a smaller denominator takes `DECISIONS.md:355` from 17/29 = 0.586 to 17/28 = 0.607.
#:   That window's text is untouched by this wave.
#:
#: **Controlled experiment, decisive.** Recomputing both trees with `catalog` held constant
#: — excluded from both (`DF_CUTOFF = 0.249`) or admitted to both (`0.2503`) — removes all
#: four of (c) and leaves only (a) and (b): 9 movers at 0.249, 10 at 0.2503 (the extra one
#: is `fr-billing-descriptors`, whose own text moved and whose direction is DF-sensitive).
#: Neutralise that single term and the scorer's contribution to this wave's histogram is
#: gone. As with the two notes below, **no wording was changed to move a score back**:
#: choosing a document's words to satisfy a corpus-wide frequency counter is standing rule
#: 6's vice from the other side.
#: Moved again 2026-08-02, fifth time that day (was 15 / 20 / 36, judged 56, total
#: unchanged at 77) after the **D-145 amendment** — the register recording that D-145's
#: "a new draft opens immediately" is false for a plan that has never published (its only
#: revision `abandoned` leaves the id spent), the owner's choice to keep the state and
#: make the refusal honest, and the new Foundation-owned `PLAN_ABANDONED_NO_SUCCESSOR`
#: (422). The deterministic layer did not move at all: `--gear gears/bss/pricing/docs
#: --auto-context` still reports 0 live and 68 suppressed, the new code is declared in
#: `design/01-foundation.md` §3.3 **and** referenced outside a block (so P3 is neutral),
#: and all three frozen stdout oracles came back **byte-identical** — the entry's added
#: lines sit at `DECISIONS.md:1220`, past every pinned P1 anchor (the highest is line 632).
#: **Six movers, and all six are one cause**, diffed per-id against the pre-edit tree
#: (`0de052f4`) in a detached worktree:
#: - `fr-billing-cycles` anchored → weak; `fr-event-contract` anchored → weak;
#:   `fr-catalogversion-increment` weak → multi; `fr-model-kind-conformance` weak → multi;
#:   `fr-billing-descriptors` multi → weak; `fr-custom-frequency` weak → anchored.
#: The cause is **`DF_CUTOFF = 0.25`**, not any of those six requirements. Exactly two
#: terms crossed it, and both were sitting inside 0.0013 of the line:
#: - **`catalog` became discriminating.** Its window count did **not** change — 568 before
#:   and after. The corpus grew from 2261 to 2272 windows (the amendment's ~66 added lines
#:   at `WINDOW_STEP = 6`), none of the 11 new windows carries the word, so its document
#:   frequency fell 568/2261 = **0.2512** (noise) to 568/2272 = **0.24999** (discriminating)
#:   and entered the term set of **29** requirements.
#: - **`rule` stopped being discriminating.** The amendment's prose states a rule and says
#:   so, six windows' worth: 565 → 571 occurrences, i.e. 565/2261 = **0.24989** to
#:   571/2272 = **0.25132**, out of the term set of **12** requirements.
#: Every one of the six then lands on `SCORE_THRESHOLD = 0.6` from one side or the other:
#: `fr-billing-cycles` gains `DESIGN.md:505-516` at 9/15 = **0.600** exactly (that window
#: already carried `catalog`; it scored 8/14 = 0.571 before) while its own anchor, which
#: does not carry the word, falls 0.571 → 0.533; `fr-event-contract`'s `design/01:73-84`
#: goes 13/22 = 0.591 → 14/23 = **0.609**; `fr-catalogversion-increment`'s two anchors both
#: go 11/19 = 0.579 → 12/20 = **0.600**; `fr-model-kind-conformance` gains two windows at
#: 10/17 = 0.588 → 11/18 = **0.611**; `fr-billing-descriptors` **loses**
#: `design/02:241-252` (`inst-ds-required`, no `catalog`) at 27/45 = **0.600** → 27/46 =
#: 0.587; and `fr-custom-frequency` loses its only account, `DECISIONS.md:349-360` (D-20,
#: carries `rule`, not `catalog`), at 18/29 = 0.621 → 17/29 = 0.586 — its term set is the
#: one that did not grow, `catalog` in and `rule` out.
#: **Controlled experiment, and it is decisive.** Recomputing both trees with `catalog` and
#: `rule` removed from every requirement's discriminating set gives **15 / 20 / 36, judged
#: 56 — the old pin — on both**, with **zero** per-id differences between them. Neutralise
#: those two terms and the amendment is invisible to this layer.
#: What the movement is **not**: an account gained or lost in the documents. None of the six
#: requirements is mentioned by the amendment, and the only moved region inside an edited
#: *window* is `DESIGN.md:505-516` — the D-72 register digest on line 508, whose matched
#: count rose by exactly one term, and that term is `catalog`, which the window already
#: carried before the edit. The wording was deliberately **left as written**, per the note
#: below: a document that says "rule" where it means rule must not be reworded because a
#: corpus-wide frequency counter is within a thousandth of a threshold.
#: Moved again 2026-08-02, fourth time that day (was 15 / 19 / 37, judged 56, total
#: unchanged at 77) after the **D-143 veto-status edit** — the register recording the
#: 2026-08-02 veto round (D-143 CONFIRMED as decided against block-and-replay; the
#: `abandon` endpoint D-145 implies put to the owner in the same round and kept). A
#: status-word edit is not supposed to move this histogram at all, and the deterministic
#: layer agrees: `--gear gears/bss/pricing/docs --auto-context` still reports 0 live and
#: 68 suppressed, and the frozen stdout oracles came back **byte-identical** — the edit
#: added no line, so not even a line number shifted. **One mover**, diffed per-id against
#: the pre-edit tree (`eb10a408`) in a detached worktree:
#: - `fr-tax-display-basis` weak → multi, on **one term**. Its declaration
#:   (`PRD.md:627`) contains "not the plan as a **whole**", so `whole` is one of its 101
#:   discriminating terms. The `DECISIONS.md:19-30` window — the register's status
#:   paragraph — already carried 60 of those 101 (**0.594**), one term under
#:   `SCORE_THRESHOLD = 0.6`; the appended veto-round sentence contains "preserves the
#:   **MUST** whole", taking it to 61/101 (**0.604**) and over. The window becomes a third
#:   candidate region and the class flips. Isolated in both directions by controlled
#:   experiment: applying the veto patch to the pre-edit tree reproduces 20 / 36 exactly
#:   and no other mover, and changing that one word to "unqualified" and nothing else
#:   restores 19 / 37.
#: What the movement is **not**: a second account of `fr-tax-display-basis`. The status
#: paragraph says nothing about tax display basis — this is a fraction-threshold artifact
#: on a common English word, the same class as the terse-prose and window-grid movers the
#: next two notes record. The wording was deliberately **left as written**: choosing a
#: document's words to push a score back under a threshold is standing rule 6's vice from
#: the other side, and would leave the register carrying a word picked by the scorer
#: rather than by the author.
#: Moved again 2026-08-02, third time that day (was 16 / 16 / 39, judged 55, total
#: unchanged at 77) after the **D-141…D-148 docs wave** — the second implementation-side
#: wave, and the first raised by writing code *against* the documents (Group G3, the
#: gear's draft-authoring plane) rather than by a lint or a review pass. **Seven movers**,
#: not the three the headline arithmetic (+3 multi, −1 anchored, −2 weak) suggests: that
#: is a net of movements in both directions. Every one was diffed per-id against the
#: pre-wave tree (3b6fa985) in a detached worktree and hand-checked against the wave's own
#: edits. Five are new accounts the wave wrote; two are losses, and both losses have a
#: cause that is not a document defect:
#: - `fr-bulk-price-import` weak → multi: D-141/D-148 rewrote the S1 §3.7 `pricing_price`
#:   bullet, which now states the per-row-ETag conflict rule and the draft-plane index's
#:   bulk consequence by name ("a batch carrying two rows on one key now fails the second
#:   **on the index**"). That window, `design/01-foundation.md:391-402`, went 0.152 →
#:   0.621 and is a second account. Its S12 account rose 0.803 → 0.864 as well (the D-141
#:   `DELETE` precondition landing in `inst-bk-phase2`).
#: - `fr-concurrent-edit` weak → multi: the declaration itself gained a whole D-141
#:   paragraph (terms 26 → 75) and two accounts appeared — the D-141 register entry
#:   (`DECISIONS.md:1189-1200`, 0.720) and the §3.7 bullet that names `fr-concurrent-edit`
#:   verbatim ("the lost update `fr-concurrent-edit` closes for `PATCH`", 0.707). Its old
#:   S12 account dropped out on the grown denominator; the id stays judged, on better
#:   evidence than before.
#: - `fr-mutation-idempotency` anchored → multi (two steps): the declaration gained two
#:   whole paragraphs (D-142 the-TTL-is-a-bound, D-143 in-flight), terms 25 → 102, and the
#:   wave's own D-142/D-143 register entries (`DECISIONS.md:1195-1206`, 0.686) plus the
#:   rewritten `pricing_idempotency_dedup` bullet (0.627 — it too names the id verbatim)
#:   are two real accounts. It had **zero** candidate accounts pre-wave, which is why it
#:   sat in `anchored:no-account` with two bare citations.
#: - `fr-pricewindow-coverage` weak → multi: declaration untouched (identical 27-term
#:   set), its S7 account unchanged at exactly 0.778, one account added — D-148's
#:   draft-plane index text states where `PriceWindow` non-overlap and coverage are
#:   enforced and how the two partial `UNIQUE` predicates coexist (0.667). Pre-wave **no**
#:   `design/01-foundation.md` window cleared 0.6 for this id; post-wave one does.
#: - `fr-published-rows-append-only` anchored → multi (two steps): declaration untouched
#:   (identical 16-term set), its three id anchors unchanged; `DESIGN.md:499-510` went
#:   0.438 → 0.688 and `DECISIONS.md:19-30` 0.438 → 0.625. Topically genuine — D-145 is
#:   exactly "flipped to a terminal `abandoned` state **instead of being deleted**" and
#:   D-148 is the canonical scope key — but **both new accounts are register-digest
#:   prose**, the DESIGN.md decision-register bullet and the DECISIONS.md status preamble,
#:   not statements of the rule. Expect `mentions` from the judge; same shape as the
#:   eighth capture's `nfr-data-residency`, which also moved on a register summary.
#: - `fr-grandfathering-eligibility` multi → **anchored:no-account** (two steps DOWN):
#:   D-147 appended three justification sentences to the FR's single-line paragraph,
#:   taking it 53 → 93 discriminating terms. Both former accounts matched **more** terms
#:   afterwards in absolute count (36 → 45 and 34 → 45) and still fell to 0.484, because a
#:   score is the *recall* of the declaration's vocabulary — the documented terse-prose
#:   limitation, same class as `fr-bundle-composition` (2026-07-31) and
#:   `fr-plan-retirement` (2026-08-01). **Not a propagation gap**: D-147 is stated in
#:   `design/07-pricewindow-linkage.md` at 236, 300 and 343, and again in DESIGN.md and
#:   `design/01-foundation.md`; P1 reports nothing against it. The added vocabulary is
#:   narrative the design has no reason to repeat (`homes`, `unreconciled`, `fault`,
#:   `glossary`, `converse`). What is lost is coverage in the *report*, not in the
#:   documents — worth `--judge-anchored` at the next N1 run.
#: - `nfr-mutation-latency` multi → weak (DOWN): **the fixed window grid, not any text
#:   about it** — the same cause as the ninth capture's `fr-level-aggregation`.
#:   Declaration untouched (identical 7-term set); the lost region
#:   `design/01-foundation.md:223-234` is **byte-identical** post-wave, merely shifted +20
#:   lines by D-144's insertion at line 175, so it no longer begins on a `6k+1` window
#:   boundary and its five matched terms split across two windows (`create/update/
#:   validation` 3/7 and `complete/within` 2/7), both under 0.6. With only 7 terms one
#:   term is decisive. Verified by controlled experiment rather than argued: inserting 20
#:   **blank** lines at the wave's own insertion point in the pre-wave tree, and changing
#:   nothing else, reproduces this mover exactly and no other.
#: Net: anchored 16 → 15 (−idempotency, −append-only, +grandfathering), multi 16 → 19
#: (+bulk-import, +concurrent-edit, +idempotency, +pricewindow-coverage, +append-only,
#: −grandfathering, −mutation-latency), weak 39 → 37 (−bulk-import, −concurrent-edit,
#: −pricewindow-coverage, +mutation-latency), judged 55 → 56, total unchanged at 77. No
#: finding appeared or vanished: `live-text.txt` and `live-json.json` are byte-identical
#: and the known-debt oracle differs only in `DECISIONS.md` line numbers, all 21 shifted
#: uniformly **+8** — one per status-board row the eight decisions added.
#: Moved again 2026-08-02, second time that day (was 17 / 38) after the D-140 docs
#: wave — the REST route-shape reconciliation (`/bss-pricing/v1/{resource}`, actions
#: as sub-resource segments). Exactly one mover, diffed per-id against the pre-wave
#: tree in a detached worktree: `fr-level-aggregation` went
#: `suspicious:multi-region` -> `suspicious:weak-coverage`, losing one of its three
#: candidate regions — the D-44 body in `DECISIONS.md`. The cause is the fixed
#: window grid, not the text: windows are cut at `WINDOW_STEP` offsets from line 1,
#: so the single **status-board row** the decision added slid the grid one line over
#: the whole register, and the window that used to carry the D-44 body's
#: discriminating terms now carries a smaller fraction of them, below the 0.6
#: threshold. Verified by controlled experiment rather than argued: adding one board
#: row to the pre-wave tree — and nothing else — reproduces this mover exactly and
#: no other. The requirement's two accounts in `design/03-price-structure.md` are
#: untouched, D-44 still sits in the register, total unchanged at 77, and no finding
#: appeared or vanished (`live-text.txt` is byte-identical; the known-debt oracle
#: differs only in DECISIONS.md line numbers).
#: Moved 2026-08-02 (was 16 / 39) after the D-139 docs wave — pricing's
#: adoption of rating T-D-25, the `capacityCharge` covered-granule factor. Exactly
#: one mover, diffed per-id against the pre-wave tree in a detached worktree:
#: `nfr-data-residency` went `suspicious:weak-coverage` -> `suspicious:multi-region`.
#: It is anchored twice in DESIGN.md (the §1.2 NFR-allocation row and §3.8), and the
#: D-72 register summary in §4 — which D-139 lengthened — sits between them, so the
#: two anchors' separation crossed the multi-region threshold. Nothing entered or
#: left the requirement set (total unchanged), no finding appeared or vanished, and
#: both classes are `suspicious:*` descriptors rather than pass/fail.
#: Moved again 2026-08-01, second time that day (was 17 / 15 / 39, judged 54, total
#: unchanged at 77), re-pinned after the 2026-08-01 slice review's fix wave
#: (D-126…D-138 + eight cleanups). Four movers, each diffed per-id against the
#: pre-wave tree (ab362b19^) and hand-checked; every one is prose movement, no
#: threshold or selection code was touched:
#: - `fr-price-history-export` **anchored → weak**: this is the previous pin note's
#:   own casualty being paid back. That note recorded it falling to anchored because
#:   "the D-125 pagination sentence added cursor/pagination vocabulary the design
#:   states once and tersely, dropping both former accounts below 0.6". Cleanup C-6
#:   (page cap vs the per-100-record SLO) expanded S12 `inst-he-export` and
#:   `dod-history-export` with exactly that vocabulary, so an account clears the
#:   threshold again and the id re-enters the judged set. The `--judge-anchored`
#:   suggestion beside it is no longer needed.
#: - `fr-audit-completeness` weak → multi: D-135 (audit-chain segmentation) rewrote
#:   S5 G4 + `inst-au-tamper` + `dod-audit` and touched S1 §3.7, lifting a second
#:   region above threshold. Stays judged.
#: - `fr-level-aggregation` weak → multi: window re-chunking from the S3
#:   `inst-tb-supersession-units` rewrite (D-127 scope clause + the D-129
#:   `included_allowance` clause) around the aggregation-field list. Stays judged.
#: - `fr-plan-retirement` multi → weak: D-128 grew the *declaration* (publish unit,
#:   pending ref, projected lifecycle field, current revision) faster than any one
#:   account, and scores are recall fractions of the declaration's vocabulary — the
#:   same class the 2026-07-31 note documents. Stays judged.
#: Net: anchored 17 → 16 (history-export leaving), multi 15 → 16 (+audit,
#: +level-aggregation, −retirement), weak unchanged at 39, judged 54 → 55.
#: Moved 2026-08-01 (was 15 / 15 / 40, judged 55, total 76), re-pinned after
#: the d-wave billing-domain review (D-123…D-125 + cleanup). Three movers, each
#: diffed per-id against the pre-wave HEAD tree and hand-checked:
#: - `nfr-observability` (new, → anchored:no-account): a fresh PRD declaration
#:   (C-5) whose id is named only by the review doc so far — no design account
#:   yet, correctly not judged; total 76 → 77.
#: - `fr-price-history-export` multi → **anchored:no-account**: the D-125
#:   pagination sentence added cursor/pagination vocabulary the design states
#:   once and tersely, dropping both former accounts below 0.6 — the documented
#:   terse-prose recall limitation, same class as fr-bundle-composition below;
#:   worth `--judge-anchored` at the next N1 run, not a document defect (S12
#:   `dod-history-export` cites the id and states the rule).
#: - `fr-grandfathering-eligibility` weak → multi: window re-chunking from the
#:   S7 `inst-co-single-pending` rewrite (C-3); stays judged.
#: Moved 2026-07-31 (was 18 / 14 / 38, judged 52), re-pinned after the day's three
#: pricing review fix rounds (D-87…D-122) rewrote fourteen requirement declarations
#: and their design accounts. All 14 movers were diffed per-id against the 7048660d
#: (pin-time) tree and the pre-c-wave HEAD before this pin was touched; every one is
#: a prose-rewrite consequence — scores are recall fractions of the declaration's
#: vocabulary, and the fix rounds changed that vocabulary; no threshold or selection
#: code moved. a/b-wave movers (11): fr-addon-rules, fr-migration-safety,
#: fr-scheduled-migration anchored→weak (their rules gained real design accounts);
#: fr-catalogversion-increment, fr-grandfathering-eligibility,
#: fr-pricewindow-coverage multi→weak; fr-billing-descriptors, fr-meter-injective,
#: fr-plan-retirement, nfr-mutation-latency weak→multi; and the one semantically
#: notable mover — fr-bundle-composition weak→**anchored:no-account**: D-104's FR
#: rewrite added evaluator/approval vocabulary the design never repeats, dropping
#: its former account below 0.6 (the documented terse-prose limitation; worth
#: `--judge-anchored` at the next N1 run). c-wave movers (3):
#: fr-consumer-readmodel-resolution anchored→weak (D-114 gave §4.4 a genuine
#: account), fr-bulk-price-import multi→weak, fr-discount-ref-hook weak→multi.
#: Moved 2026-07-30, multi-region 3 -> 5 and weak-coverage 14 -> 12, after the five
#: document defects the two N1 evaluations found were fixed. Both movers were checked
#: by hand before this pin was touched, which is the only reason to touch it:
#:
#:   fr-allocation-precedence          weak-coverage -> multi-region
#:   fr-manual-adjustment-governance   weak-coverage -> multi-region
#:
#: Neither is a false account. `fr-allocation-precedence` gained `03:145-156` (§1.6 P4)
#: and `03:793-804` (§11.2), both of which already said "statutory registry deferred,
#: out of v1 scope" — they cleared the bar only because rewriting the stale PRD
#: declaration to match them brought the shared vocabulary up. The rule was stated in
#: three places all along; the PRD was the lone dissenter. `fr-manual-adjustment-
#: governance` gained `05:115-126`, the new §1.6 D5 row recording the unresolved
#: before/after-audit question — genuinely about the requirement, and a judge should
#: read it as `mentions`, since raising a question is not stating a rule.
#:
#: The judged total is unchanged at 17 (5 + 12), so the ladder's budget did not move.
PINNED_TRIAGE_LEDGER = {
    "unbuildable:no-prose": 0,
    "no-region": 0,
    "anchored:no-account": 23,
    "suspicious:multi-region": 5,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 12,
    "covered:strong": 0,
}

#: Judge calls the pinned histograms imply: 56 + 17. Pinned separately because it is
#: the number the ladder exists to control, and a threshold change that quietly
#: doubles it must read as a diff. (Pricing 52 until 2026-07-31 — the day's three
#: review fix rounds moved the histogram, note above; +3 is document movement, not
#: ladder drift. 55 → 54 on the d-wave re-pin, then back to 55 on the 2026-08-01
#: fix-wave re-pin when C-6 repaired the terse-prose account that had cost
#: `fr-price-history-export` its judged slot — both movements are documents, and
#: both are itemised per-id in the notes above. 55 → 56 on the 2026-08-02
#: D-141…D-148 re-pin: four ids entered the judged set — `fr-mutation-idempotency`
#: and `fr-published-rows-append-only` promoted out of `anchored:no-account`, and
#: `fr-grandfathering-eligibility` demoted into it — a +1 net over seven movers, all
#: of them documents and all itemised per-id above. 56 → 57 on the 2026-08-02 D-145
#: amendment re-pin: three ids entered the judged set and two left it, all six movers
#: driven by the single `catalog`/`rule` document-frequency swap itemised above — not
#: ladder drift, and not a document defect either, which the controlled run recorded there
#: settles by reproducing the old pin exactly once those two terms are neutralised.)
#: (57 → 58 on the 2026-08-03 D-149…D-154 re-pin. The judged share moves by exactly the
#: one requirement that left an unjudged class: `nfr-size-limits`, `no-region` → judged
#: `weak-coverage`, having gained its first candidate region ever. Every other mover of
#: that wave is judged-to-judged — `multi-region` and `weak-coverage` are both in `JUDGED`,
#: so the +7/−6 between them is judge-neutral. Per-id record beside
#: `PINNED_TRIAGE_PRICING`.)
#: (58 → 62 on the 2026-08-03 D-155…D-161 re-pin — the second re-pin of that day and the
#: largest single jump the judged share has taken. **Four** ids entered the judged set and
#: none left it, all four promoted out of `anchored:no-account`:
#: `fr-customer-group-pricing`, `fr-event-contract`, `fr-grandfathering-eligibility` and
#: `fr-pricing-snapshot`. Only one of the four — `fr-pricing-snapshot`, on D-161's entry —
#: was promoted by an account about its own subject; `fr-event-contract` is the `catalog`
#: document-frequency artifact with **no new region at all**, and the other two gained the
#: D-72 register digest or the wave preamble and nothing else. The +12/−8 between
#: `multi-region` and `weak-coverage` is judge-neutral, both classes being in `JUDGED`. The
#: judged share is now 62/77 = 81%, which is worth watching rather than accepting: standing
#: rule 4 says a class that stops being a minority is a statement about the search, and three
#: consecutive waves have grown the two documents every requirement's terms can match. Per-id
#: record beside `PINNED_TRIAGE_PRICING`.)
#: (62 → 64 on the 2026-08-03 D-162 re-pin, the third of that day. **Two** ids entered the
#: judged set and none left it: `nfr-read-latency` out of `no-region` and
#: `fr-bundle-composition` out of `anchored:no-account`. Neither was promoted by an account
#: about its own subject — both gained the D-72 register digest and nothing else, one of them
#: its first candidate region ever. The +4/−2 between `multi-region` and `weak-coverage` is
#: judge-neutral. **The share is 64/77 = 83% and this is the fourth consecutive wave to grow
#: it without a design slice moving**, which the D-162 controlled run demonstrates outright
#: rather than by inference: replacing only `DESIGN.md` and `DECISIONS.md` in the pre-edit
#: tree reproduces every mover, and the design-slice and PRD edits reproduce none. The digest
#: D-72 obliges a wave to keep current summarises every requirement in the gear, so every
#: requirement's vocabulary can match it — which is a property of the corpus, not of the
#: coverage. Per-id record beside `PINNED_TRIAGE_PRICING`.)
#: 69 -> 67 on the 2026-08-08 D-259 tail wave. The triage histogram is unchanged,
#: so this is the judged-share side of the same knife-edge movement recorded above,
#: moving for the third time in one day on prose that touched neither requirement.
PINNED_JUDGE_CALLS = {"pricing": 70, "ledger": 17}
#: pricing 67 -> 70 on 2026-08-14, and **not by D-312** — same control as
#: `PINNED_TRIAGE_PRICING` above: measured against 3492f0091, the judged set is
#: unchanged across D-312's edits. The movement belongs to the four waves that
#: landed between the D-291 capture and this one.
#: pricing 69 -> 67 on the 2026-08-08 **D-251/D-252/D-234 propagation wave**. Two movers,
#: `fr-scheduled-migration` and `fr-supersession`, both leaving the judged set for
#: `anchored:no-account` — the full control is recorded beside `PINNED_TRIAGE_PRICING` above and
#: it is document-frequency movement over regions no edit of this wave touched, not ladder drift.
#: `fr-supersession` is here for the second time on the same mechanism; see the 2026-08-05 note
#: below, where its one account scored exactly 0.600 and this time 0.602.
#: pricing 70 -> 69 on the 2026-08-06 **Slice 9 overlay merge docs wave**. One mover,
#: `fr-event-contract`, leaving the judged set for `anchored:no-account` — the full control is
#: recorded beside `PINNED_TRIAGE_PRICING` above and it is document-frequency movement over a
#: byte-identical set of regions, not ladder drift.
#: pricing 69 -> 68 on the 2026-08-05 **D-183…D-193 phase-4 close docs wave**. One mover,
#: `fr-supersession`, whose only account (`design/01-foundation.md`, `term-overlap`) scored
#: **exactly 0.600** and fell under the threshold when corpus growth changed the
#: discriminating-term set. Its document was not edited by the wave and its prose account
#: still exists — see the straddler analysis beside `PINNED_TRIAGE_PRICING`.
#: pricing 68 -> 69 on the 2026-08-03 **phase-closing docs wave** (D-180/D-181 plus two id-less
#: records of what the phase proved by execution). **One net id entered the judged set**:
#: `nfr-event-propagation`, out of `no-region`, which is not judged, into `weak-coverage`, which
#: is. The other five movers are `weak-coverage` -> `multi-region`, both judged, so they are
#: judge-neutral and change only the histogram's shape. Not one of the six is an account: each
#: gains a single `term-overlap` region at 0.600–0.667 in one of the corpus's two mega-windows,
#: on one to four terms of generic vocabulary. `nfr-event-propagation`'s own increment is the
#: word **`events`**, from a clause naming the four gears `pricing_outbox` emits to. Unusually,
#: the blank-line control does **not** reproduce this move — the added words are the cause, and
#: they are still not an account. Full per-id decomposition, including the single `DF_CUTOFF`
#: crossing (`unit`, the known oscillator, back outside), beside `PINNED_TRIAGE_PRICING`.
#: pricing 67 -> 68 on the 2026-08-03 **D-179** wave (phase 3, G1). **One** id entered the
#: judged set and none left it: `fr-supersession`, out of `anchored:no-account`, which is not a
#: judged class, into `weak-coverage`, which is. It is the same threshold-adjacent id that left
#: the judged set one capture ago at 0.6091 -> 0.5676, returning at **0.604**, and a controlled
#: run with 13 **blank** lines at this wave's insertion point reproduces it identically — so the
#: increment is the window grid and not an account. See the note beside
#: `PINNED_TRIAGE_PRICING`.
#: pricing 68 -> 69 on the 2026-08-03 D-170…D-174 wave (G7, the REST surface), the sixth move
#: of that day. **One** id entered the judged set and none left it: `nfr-event-propagation`,
#: out of `no-region`; the +3/−2 between `multi-region` and `weak-coverage` is judge-neutral.
#: **The first increment in seven waves that the D-72 digest and the register preamble did not
#: produce** — a controlled run with those two documents held at their pre-wave content
#: reproduces this mover and only this mover, and a run with *only* those two reproduces the
#: other three and not this one. Its cause is still not an account: one term, `within`, from a
#: rejected-alternative clause in the `pricing_idempotency_dedup` bullet, tipping a window whose
#: real relevance (`pricing_outbox`) this wave never touched. 69/77 = 90%. Per-id record beside
#: `PINNED_TRIAGE_PRICING`.
#: pricing 64 -> 67 on 2026-08-03 with the D-163…D-168 wave: three ids promoted out of
#: `anchored:no-account` (`fr-publish-fanout-atomicity`, `fr-supersession`,
#: `nfr-observability`), the `multi`/`weak` traffic between them being judge-neutral.
#: Per-id record beside `PINNED_TRIAGE_PRICING`, including why only one of the three is
#: an account this wave wrote.
#: (67 -> 68 on the 2026-08-03 D-169 re-pin, the fifth of that day and the smallest edit yet
#: to move this number: **one** id entered the judged set, `fr-per-seat`, out of
#: `anchored:no-account`, and none left it; the +3/−2 between `multi-region` and
#: `weak-coverage` is judge-neutral. It was promoted by the D-72 digest gaining the single
#: term `wording` — not by any account of per-seat pricing, of which the digest contains
#: none. **68/77 = 88%, the sixth consecutive wave to grow the share without a design slice
#: moving**, and the controlled run here is the strongest form of that statement so far: the
#: whole decision — register entry, board row, closed fork row, five Slice-6 edits, two PRD
#: edits — reproduces **zero** movers, and the two obligatory summary documents reproduce all
#: four. What is being counted as coverage is a digest that summarises every requirement in
#: the gear, so every requirement's vocabulary can match it. Per-id record beside
#: `PINNED_TRIAGE_PRICING`.)


#: pricing 68 -> 70 on 2026-08-05 with the D-88/D-195 supersession wave (D-196 + D-197 minted).
#: Two ids left `anchored:no-account` for `suspicious:weak-coverage` and are the whole +2:
#: `fr-supersession` and `fr-rating-compatibility`. A third mover, `nfr-audit-retention`
#: (`weak-coverage` -> `multi-region`), is judge-neutral. Total held at 77.
#: **Why this is a real promotion and not the digest coincidence the four movers above were.**
#: Every previous mover on this list was caused by a summary document gaining a term — one of
#: them by the single word `wording` — with no account of the requirement written anywhere. This
#: wave wrote the supersession unit: `inst-su-compose` and `inst-su-commit` gained the composed
#: window operations, the refusal ordering, the two-door occupancy reading and the commit's
#: write ordering, and D-195/D-196 discuss the shape at length. `fr-rating-compatibility` follows
#: because D-196's subject **is** whether the per-`(meter, dimensionKey)` line model rating
#: presupposes is storable at all — the negative account is still an account.
#: **What was checked before accepting it:** the live-findings count stayed at **2** and the
#: known-debt count at **59**, so nothing closed silently — which is the hazard the 2026-08-05
#: `WINDOW_GAP` episode recorded, where a bare code token in register prose satisfied P3's
#: "referenced by" test and moved 59 -> 58 without paying the debt. Here neither count moved,
#: and the per-id diff was taken against a stashed pre-wave tree rather than inferred.


def test_pricing_triage_histogram_is_pinned(tmp_path):
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--out", str(out)).returncode == 0
    counts = json.loads(out.read_text(encoding="utf-8"))["counts"]
    assert counts == PINNED_TRIAGE_PRICING
    assert sum(counts.values()) == 77


def test_ledger_triage_histogram_is_pinned(tmp_path):
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/ledger/docs", "--out", str(out)).returncode == 0
    counts = json.loads(out.read_text(encoding="utf-8"))["counts"]
    assert counts == PINNED_TRIAGE_LEDGER
    assert sum(counts.values()) == 40


def test_the_judged_share_stays_a_minority(tmp_path):
    # The ladder's purpose. It once judged 110 of 116 because any id anchor counted
    # towards multiplicity; a citation cannot contradict anything, so it does not.
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--out", str(out)).returncode == 0
    counts = json.loads(out.read_text(encoding="utf-8"))["counts"]
    from spec_check.semantic import triage
    judged = sum(counts[name] for name in counts if name in triage.JUDGED)
    assert judged == PINNED_JUDGE_CALLS["pricing"]
    assert judged < sum(counts.values())


def test_the_ladder_reaches_more_than_one_class(tmp_path):
    # The guard against the failure this configuration replaced: with an absolute
    # term-count threshold every requirement was `multi-region`, three of the six
    # classes were unreachable, and the triage spent a judge call on all 116. A
    # histogram with one populated class is a broken triage, not a clean corpus.
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--gear", "gears/bss/ledger/docs",
               "--out", str(out)).returncode == 0
    counts = json.loads(out.read_text(encoding="utf-8"))["counts"]
    assert len([name for name, count in counts.items() if count]) >= 3
