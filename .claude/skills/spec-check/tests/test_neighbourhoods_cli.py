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
    assert envelope["thresholds"]["score_threshold"] == 4
    assert envelope["thresholds"]["strong_score"] == 8
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


# The pinned live triage histogram the plan asks for (task 5, steps 7-8) is
# deliberately NOT here yet. The first live run puts 116 of 116 requirements in
# `suspicious:multi-region`, which makes three of the six classes unreachable —
# see docs/superpowers/plans/2026-07-30-spec-check-n1-requirements.md and the
# measurement recorded with it. Pinning that histogram would freeze a degenerate
# triage and call it a baseline, which is exactly what a pin is supposed to
# prevent.
