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
#: strong 0.75. Not the first output of the code: the first run (absolute term
#: counts, threshold 4) put 116 of 116 requirements in `suspicious:multi-region`
#: and was rejected as degenerate rather than pinned.
#:
#: What was checked by eye, per the plan's procedure:
#:
#: - `no-region` × 6, all of them pricing NFRs (read latency, event propagation,
#:   multi-currency scale, mass-repricing throughput, availability/DR, size
#:   limits). Cross-checked negatively: the design slices mention `p95 < 100ms` in
#:   ten places, every one of them a reference to a budget the PRD defines, and no
#:   slice specifies an SLO. So the class is reporting what it says it reports —
#:   either unaddressed, or stated in other words — and here it is the former.
#: - `suspicious:weak-coverage` × 10, three read: `fr-billing-cycles` scores 0.571
#:   on its single anchored region, `fr-bundle-composition` 0.333, and
#:   `fr-addon-rules` 0.019 — one term of 53. The last is exactly the shape the
#:   design predicted: the id is named and the rule is not there.
#: - `suspicious:multi-region`, one read in full: ledger's `fr-idempotency-per-flow`
#:   is anchored in four distinct slices scoring 0.000, 0.571, 0.143 and 0.286 —
#:   distinct locations, and three of the four name the id while saying nothing.
#: - `covered:strong` × 0 and `not-normative` × 0 are honest zeroes, not gaps:
#:   `covered:strong` needs *exactly one* region and a requirement named in one
#:   slice usually also has a term-overlap region, while requirement prose in both
#:   corpora is overwhelmingly normative. Both stay in the histogram at zero so a
#:   class that stops occurring cannot be mistaken for one that never existed.
PINNED_TRIAGE_PRICING = {
    "unbuildable:no-prose": 0,
    "no-region": 6,
    "suspicious:multi-region": 60,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 10,
    "covered:strong": 0,
}
PINNED_TRIAGE_LEDGER = {
    "unbuildable:no-prose": 0,
    "no-region": 0,
    "suspicious:multi-region": 37,
    "suspicious:not-normative": 0,
    "suspicious:weak-coverage": 3,
    "covered:strong": 0,
}


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
