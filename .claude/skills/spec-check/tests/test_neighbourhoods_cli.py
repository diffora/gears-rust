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
PINNED_TRIAGE_PRICING = {
    "unbuildable:no-prose": 0,
    "no-region": 4,
    "anchored:no-account": 9,
    "suspicious:multi-region": 44,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 20,
    "covered:strong": 0,
}
#: Moved 2026-08-03, **third move of the day** (was 5 / 10 / 40 / 22, judged 62, total
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
PINNED_JUDGE_CALLS = {"pricing": 64, "ledger": 17}


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
