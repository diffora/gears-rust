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
    assert len(ids) == 76
    assert len(set(ids)) == 76
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
    "no-region": 6,
    "anchored:no-account": 15,
    "suspicious:multi-region": 15,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 40,
    "covered:strong": 0,
}
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

#: Judge calls the pinned histograms imply: 55 + 17. Pinned separately because it is
#: the number the ladder exists to control, and a threshold change that quietly
#: doubles it must read as a diff. (Pricing 52 until 2026-07-31 — the day's three
#: review fix rounds moved the histogram, note above; +3 is document movement, not
#: ladder drift.)
PINNED_JUDGE_CALLS = {"pricing": 55, "ledger": 17}


def test_pricing_triage_histogram_is_pinned(tmp_path):
    out = tmp_path / "n.json"
    assert run("--gear", "gears/bss/pricing/docs", "--out", str(out)).returncode == 0
    counts = json.loads(out.read_text(encoding="utf-8"))["counts"]
    assert counts == PINNED_TRIAGE_PRICING
    assert sum(counts.values()) == 76


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
