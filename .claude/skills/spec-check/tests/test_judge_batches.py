import json
import subprocess
import sys

from conftest import REPO_ROOT, SCRIPTS

JUDGE_BATCHES_PY = SCRIPTS / "judge_batches.py"
sys.path.insert(0, str(SCRIPTS))

import judge_batches  # noqa: E402


def item(ident, files, judge=True, declared_at=1):
    return {
        "id": "requirement/{}".format(ident),
        "requirement_id": ident,
        "judge": judge,
        "triage": "suspicious:multi-region" if judge else "anchored:no-account",
        "fragments": [
            {"role": "requirement-declaration", "file": "PRD.md",
             "lines": [declared_at, declared_at + 1],
             "text": "Publish **MUST** freeze."},
        ] + [
            {"role": "candidate-region", "file": path, "lines": [10, 21],
             "text": "prose of " + path, "selected_by": "id-anchor", "score": 0.8,
             "matched_terms": 8}
            for path in files
        ],
    }


def test_batches_never_quote_overlapping_lines():
    # The mitigation for batching at all: a judgment about one paragraph cannot be
    # carried into a verdict about another requirement quoting that same paragraph.
    members = [
        item("fr-a", ["design/01.md"], declared_at=1),
        item("fr-b", ["design/01.md"], declared_at=50),
        item("fr-c", ["design/02.md"], declared_at=99),
        item("fr-d", ["design/03.md"], declared_at=150),
    ]
    batches = judge_batches.group(members, 4)
    for batch in batches:
        seen = []
        for member in batch:
            spans = judge_batches._spans(member)
            assert not judge_batches._overlaps(spans, seen)
            seen.extend(spans)
    # a and b both quote design/01.md:10-21, so they cannot share a batch
    placement = {m["id"]: n for n, b in enumerate(batches) for m in b}
    assert placement["requirement/fr-a"] != placement["requirement/fr-b"]


def test_two_requirements_of_one_prd_can_share_a_batch():
    # Comparing whole files instead of line spans sounds stricter and saves
    # nothing: every requirement of a gear declares itself in the same PRD.md, so
    # every pair would conflict and every batch would hold one member. Measured on
    # ledger, that turned 17 judged neighbourhoods into 17 dispatches.
    members = [
        item("fr-a", ["design/01.md"], declared_at=10),
        item("fr-b", ["design/02.md"], declared_at=400),
    ]
    assert len(judge_batches.group(members, 4)) == 1


def test_every_judged_neighbourhood_lands_in_exactly_one_batch():
    members = [item("fr-{}".format(n), ["design/{:02d}.md".format(n)], declared_at=1 + n * 20)
               for n in range(9)]
    batches = judge_batches.group(members, 4)
    flat = [m["id"] for batch in batches for m in batch]
    assert sorted(flat) == sorted(m["id"] for m in members)
    assert len(flat) == len(set(flat))


def test_a_neighbourhood_that_conflicts_with_everything_goes_alone():
    members = [item("fr-a", ["design/01.md"], declared_at=1),
               item("fr-b", ["design/01.md"], declared_at=50),
               item("fr-c", ["design/01.md"], declared_at=99)]
    batches = judge_batches.group(members, 4)
    assert [len(b) for b in batches] == [1, 1, 1]


def test_grouping_is_deterministic():
    members = [item("fr-{}".format(n), ["design/{:02d}.md".format(n % 3)], declared_at=1 + n * 20)
               for n in range(6)]
    first = [[m["id"] for m in b] for b in judge_batches.group(members, 3)]
    second = [[m["id"] for m in b] for b in judge_batches.group(members, 3)]
    assert first == second


def test_a_batch_prompt_hides_selection_provenance():
    # Batching must not become a hole in the control the single-dispatch path holds.
    text = judge_batches.render_batch([item("fr-a", ["design/01.md"]),
                                       item("fr-b", ["design/02.md"])])
    for hidden in ("id-anchor", "term-overlap", "selected_by", "score", "matched_terms",
                   "multi-region", "triage"):
        assert hidden not in text
    assert "JSON **array** of 2 objects" in text
    assert "=== requirement/fr-a ===" in text
    assert "design/01.md:10-21" in text


def test_a_single_member_batch_asks_for_one_object_not_an_array():
    text = judge_batches.render_batch([item("fr-a", ["design/01.md"])])
    assert "single JSON object" in text
    assert "array" not in text


def test_cli_writes_one_file_per_batch_plus_a_manifest(tmp_path):
    envelope = {
        "gears": ["gears/bss/ledger/docs"],
        "thresholds": {},
        "counts": {},
        "neighbourhoods": [
            item("fr-a", ["design/01.md"], declared_at=1),
            item("fr-b", ["design/02.md"], declared_at=50),
            item("fr-c", ["design/03.md"], judge=False, declared_at=99),
        ],
    }
    nb_path = tmp_path / "n.json"
    nb_path.write_text(json.dumps(envelope), encoding="utf-8")
    out_dir = tmp_path / "batches"
    proc = subprocess.run(
        [sys.executable, str(JUDGE_BATCHES_PY), "--neighbourhoods", str(nb_path),
         "--out-dir", str(out_dir), "--size", "1"],
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    assert proc.returncode == 0, proc.stderr
    assert proc.stderr == ""
    manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
    assert [entry["ids"] for entry in manifest["batches"]] == [
        ["requirement/fr-a"], ["requirement/fr-b"],
    ]
    assert (out_dir / "batch-01.md").exists()
    assert (out_dir / "batch-02.md").exists()
    assert not (out_dir / "batch-03.md").exists()  # the unjudged one is not dispatched
    # The count of what is *not* dispatched is printed, not silently dropped.
    assert "not judged: 1 neighbourhood(s)" in proc.stdout


def test_cli_rejects_a_missing_file(tmp_path):
    proc = subprocess.run(
        [sys.executable, str(JUDGE_BATCHES_PY), "--neighbourhoods", str(tmp_path / "nope.json"),
         "--out-dir", str(tmp_path / "b")],
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    assert proc.returncode == 1
    assert proc.stderr.startswith("Error: ")


def test_the_live_ledger_corpus_batches_into_far_fewer_dispatches(tmp_path):
    # The whole point, measured on the corpus rather than asserted in prose.
    envelope = json.loads(
        (REPO_ROOT / ".spec-check/neighbourhoods-ledger.json").read_text(encoding="utf-8")
    ) if (REPO_ROOT / ".spec-check/neighbourhoods-ledger.json").exists() else None
    if envelope is None:  # the artifact is git-ignored; regenerate it to run this
        import pytest
        pytest.skip("run neighbourhoods.py --gear gears/bss/ledger/docs first")
    judged = [n for n in envelope["neighbourhoods"] if n["judge"]]
    batches = judge_batches.group(judged, 4)
    # Batching must actually batch. It once did not: comparing files rather than
    # line spans produced one dispatch per neighbourhood on this very corpus.
    assert len(batches) <= (len(judged) + 1) // 2


def test_both_prompt_templates_demand_the_prefixed_id():
    # The judge returned bare ids on every single-requirement batch of the ledger
    # step-1 run, because only the multi template asked for the header form. Every
    # verdict then had to be renormalised by hand before the report would render.
    single = judge_batches.render_batch([item("fr-a", ["design/01.md"])])
    multi = judge_batches.render_batch([item("fr-a", ["design/01.md"]),
                                        item("fr-b", ["design/02.md"])])
    for text in (single, multi):
        assert "`requirement/` prefix" in text
        assert "matches verdicts to neighbourhoods on that exact string" in text
