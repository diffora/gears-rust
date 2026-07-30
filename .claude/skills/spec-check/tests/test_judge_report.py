import json
import subprocess
import sys

from conftest import REPO_ROOT, SCRIPTS

JUDGE_REPORT_PY = SCRIPTS / "judge_report.py"

FRAGMENTS = [
    {"role": "requirement-declaration", "file": "PRD.md", "lines": [10, 12],
     "text": "Publish **MUST** freeze the snapshot."},
    {"role": "candidate-region", "file": "design/03-c.md", "lines": [88, 99],
     "text": "Publishing freezes the snapshot.", "selected_by": "id-anchor", "score": 9},
    {"role": "candidate-region", "file": "design/09-i.md", "lines": [145, 156],
     "text": "Publishing recomputes the snapshot.", "selected_by": "term-overlap", "score": 6},
]


def envelope(*neighbourhoods):
    return {
        "gears": ["gears/bss/ledger/docs"],
        "thresholds": {"window_lines": 12, "window_step": 6, "score_threshold": 4,
                       "strong_score": 8, "document_frequency_cutoff": 0.25,
                       "max_anchors": 4, "max_overlap_regions": 3, "max_fragments": 6},
        "counts": {},
        "neighbourhoods": list(neighbourhoods),
    }


def judged(triage="suspicious:multi-region", ident="cpt-cf-bss-ledger-fr-thing"):
    return {
        "id": "requirement/{}".format(ident), "kind": "requirement", "gear": "ledger",
        "requirement_id": ident, "requirement_kind": "fr", "priority": "p1",
        "declaration_line": 10,
        "triage": triage, "judge": True, "fragments": FRAGMENTS, "unbuildable": [],
    }


def verdict(**overrides):
    base = {
        "id": "requirement/cpt-cf-bss-ledger-fr-thing",
        "regions": [
            {"file": "design/03-c.md", "lines": [88, 99],
             "role": "specifies", "usefulness": "decisive"},
            {"file": "design/09-i.md", "lines": [145, 156],
             "role": "specifies", "usefulness": "useful"},
        ],
        "coverage": "specified",
        "agreement": "divergent",
        "citations": [
            {"file": "design/03-c.md", "line": 92, "quote": "freezes the snapshot"},
            {"file": "design/09-i.md", "line": 151, "quote": "recomputes the snapshot"},
        ],
        "reasoning": "One freezes, the other recomputes.",
        "proposed_fix": "design/09-i.md must stop recomputing the snapshot.",
    }
    base.update(overrides)
    return base


def run(tmp_path, envelope_obj, verdicts, out_name="N1-ledger.md"):
    tmp_path.mkdir(parents=True, exist_ok=True)
    nb_path = tmp_path / "n.json"
    v_path = tmp_path / "v.json"
    out = tmp_path / out_name
    nb_path.write_text(json.dumps(envelope_obj), encoding="utf-8")
    v_path.write_text(json.dumps(verdicts), encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(JUDGE_REPORT_PY),
         "--neighbourhoods", str(nb_path), "--verdicts", str(v_path), "--out", str(out)],
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    text = out.read_text(encoding="utf-8") if out.exists() else ""
    return proc, text


def test_a_well_formed_divergent_verdict_is_reported_with_both_sides(tmp_path):
    proc, report = run(tmp_path, envelope(judged()), [verdict()])
    assert proc.returncode == 0, proc.stderr
    assert "| fr-thing | divergent | design/03-c.md:92 | design/09-i.md:151 |" in report
    assert "must stop recomputing" in report
    assert "](" not in report  # plain path:line text, never a markdown link


def test_divergent_without_two_sides_is_downgraded_and_the_downgrade_is_recorded(tmp_path):
    # Honesty rule 2, machine-enforced: the rule stops depending on the prompt.
    one_side = verdict(citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes the snapshot"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [one_side])
    assert proc.returncode == 0
    assert "| fr-thing | specified |" in report
    assert "Downgrades" in report
    assert "divergent → consistent" in report


def test_two_citations_in_one_location_are_not_two_sides(tmp_path):
    same_place = verdict(citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes"},
        {"file": "design/03-c.md", "line": 92, "quote": "the snapshot"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [same_place])
    assert "divergent → consistent" in report
    assert "| fr-thing | divergent |" not in report


def test_agreement_needs_two_specifies_regions(tmp_path):
    one_specifies = verdict(
        regions=[
            {"file": "design/03-c.md", "lines": [88, 99],
             "role": "specifies", "usefulness": "decisive"},
            {"file": "design/09-i.md", "lines": [145, 156],
             "role": "mentions", "usefulness": "noise"},
        ],
        agreement="consistent",
        proposed_fix="design/09-i.md should cite the requirement it merely mentions.",
    )
    proc, report = run(tmp_path, envelope(judged()), [one_specifies])
    assert "not-applicable" in report
    assert "consistent → not-applicable" in report


def test_a_citation_outside_every_fragment_is_a_judge_failure(tmp_path):
    # The mechanical stand-in for "no repository access": a judge that answered
    # from the repository cites a line that was never in its neighbourhood.
    smuggled = verdict(citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes the snapshot"},
        {"file": "design/11-lifecycle.md", "line": 400, "quote": "never shown to the judge"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [smuggled])
    assert "| fr-thing | judge-failed |" in report
    assert "outside every fragment" in report


def test_a_defect_verdict_without_a_proposed_fix_is_a_judge_failure(tmp_path):
    no_fix = verdict(coverage="claim-only", agreement="not-applicable", proposed_fix="")
    proc, report = run(tmp_path, envelope(judged()), [no_fix])
    assert "| fr-thing | judge-failed |" in report
    assert "proposed_fix" in report


def test_a_specified_consistent_verdict_needs_no_proposed_fix(tmp_path):
    clean = verdict(agreement="consistent", proposed_fix="", citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes the snapshot"},
        {"file": "design/09-i.md", "line": 151, "quote": "freezes it too"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [clean])
    assert "| fr-thing | specified |" in report
    assert "judge-failed" not in report


def test_a_malformed_verdict_is_recorded_not_dropped(tmp_path):
    proc, report = run(
        tmp_path, envelope(judged()), [{"id": "requirement/cpt-cf-bss-ledger-fr-thing"}]
    )
    assert proc.returncode == 0
    assert "| fr-thing | judge-failed |" in report


def test_a_missing_verdict_is_recorded_not_dropped(tmp_path):
    proc, report = run(tmp_path, envelope(judged()), [])
    assert proc.returncode == 0
    assert "| fr-thing | judge-failed |" in report
    assert "no verdict" in report


def test_unknown_coverage_value_is_a_judge_failure(tmp_path):
    proc, report = run(tmp_path, envelope(judged()), [verdict(coverage="probably-fine")])
    assert "| fr-thing | judge-failed |" in report


def test_deterministic_classes_are_reported_with_their_reason(tmp_path):
    from spec_check.semantic import neighbourhood as nb

    no_prose = judged(triage="unbuildable:no-prose", ident="cpt-cf-bss-ledger-fr-bare")
    no_prose["judge"] = False
    no_prose["fragments"] = []
    no_prose["unbuildable"] = [nb.UNBUILDABLE_REASONS["unbuildable:no-prose"]]

    no_region = judged(triage="no-region", ident="cpt-cf-bss-ledger-fr-lonely")
    no_region["judge"] = False
    no_region["fragments"] = FRAGMENTS[:1]
    no_region["unbuildable"] = [nb.UNBUILDABLE_REASONS["no-region"]]

    proc, report = run(tmp_path, envelope(no_prose, no_region), [])
    assert proc.returncode == 0
    assert "| fr-bare | " in report
    assert "| fr-lonely | " in report
    assert "either it is unaddressed, or the design states it in different words" in report
    # Honesty rule 1: not judged is not the same as not reported.
    assert "judge-failed" not in report
    # The no-prose row has no fragment to cite, so it names the declaration line.
    assert "PRD.md:10" in report


def test_covered_strong_is_listed_with_the_reason_it_was_not_judged(tmp_path):
    strong = judged(triage="covered:strong", ident="cpt-cf-bss-ledger-fr-solid")
    strong["judge"] = False
    proc, report = run(tmp_path, envelope(strong), [])
    assert "fr-solid" in report
    assert "Not judged — covered" in report
    assert "judge-failed" not in report


def _contradicts(**overrides):
    """A verdict whose single account contradicts the declaration itself."""
    base = verdict(
        agreement="contradicts-declaration",
        regions=[{"file": "design/03-c.md", "lines": [88, 99],
                  "role": "specifies", "usefulness": "decisive"}],
        citations=[
            {"file": "PRD.md", "line": 11, "quote": "Publish MUST freeze the snapshot"},
            {"file": "design/03-c.md", "line": 92, "quote": "freezing is out of v1 scope"},
        ],
        proposed_fix="PRD.md must be scoped to match design/03-c.md, or the design must commit.",
    )
    base.update(overrides)
    return base


def test_a_declaration_contradiction_is_reported_with_both_sides(tmp_path):
    # The axis N1 lacked: `agreement` compares accounts with each other, so a design
    # that explicitly declines a PRD MUST had nowhere to land and fell into
    # `underspecified`. Measured twice in a sample of twelve.
    proc, report = run(tmp_path, envelope(judged()), [_contradicts()])
    assert "| fr-thing | contradicts-declaration |" in report
    assert "PRD.md:11" in report and "design/03-c.md:92" in report
    assert "judge-failed" not in report


def test_a_declaration_contradiction_survives_having_one_account(tmp_path):
    # It must not be swept up by the "fewer than two `specifies` → not-applicable"
    # rule: the contradiction is between one account and side A, not between accounts.
    proc, report = run(tmp_path, envelope(judged()), [_contradicts()])
    assert "not-applicable" not in report


def test_a_declaration_contradiction_needs_a_citation_in_the_declaration(tmp_path):
    # Both sides cited inside design regions proves accounts disagree with each other,
    # which is `divergent`, not this. Without the declaration side the claim is unshown.
    no_side_a = _contradicts(citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes"},
        {"file": "design/09-i.md", "line": 151, "quote": "recomputes"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [no_side_a])
    assert "| fr-thing | contradicts-declaration |" not in report
    assert "contradicts-declaration → not-applicable" in report


def test_a_declaration_contradiction_needs_a_citation_outside_it(tmp_path):
    # Citing only the declaration shows nothing contradicting it.
    only_side_a = _contradicts(citations=[
        {"file": "PRD.md", "line": 10, "quote": "Publish"},
        {"file": "PRD.md", "line": 11, "quote": "MUST freeze"},
    ])
    proc, report = run(tmp_path, envelope(judged()), [only_side_a])
    assert "| fr-thing | contradicts-declaration |" not in report
    assert "contradicts-declaration → not-applicable" in report


def test_anchored_no_account_is_not_reported_as_covered(tmp_path):
    # These two classes are opposites — `covered:strong` means one account cleared
    # the strong threshold, `anchored:no-account` means nothing cleared the account
    # bar at all. Sharing a branch made the report assert the reverse of the truth
    # for 10 of the 16 requirements in the ledger step-1 run.
    bare = judged(triage="anchored:no-account", ident="cpt-cf-bss-ledger-fr-named")
    bare["judge"] = False
    proc, report = run(tmp_path, envelope(bare), [])
    assert "Not judged — anchored, no account" in report
    assert "Not judged — covered" not in report
    assert "at or above the strong threshold" not in report
    assert "| no-account | 1 |" in report


def test_a_specified_consistent_verdict_may_omit_proposed_fix_entirely(tmp_path):
    # Distinct from the empty-string case above: the agent contract calls the key
    # optional here, so its absence must be accepted exactly as an empty value is.
    clean = verdict(agreement="consistent", citations=[
        {"file": "design/03-c.md", "line": 92, "quote": "freezes the snapshot"},
        {"file": "design/09-i.md", "line": 151, "quote": "freezes it too"},
    ])
    del clean["proposed_fix"]
    proc, report = run(tmp_path, envelope(judged()), [clean])
    assert "| fr-thing | specified |" in report
    assert "judge-failed" not in report


def test_a_defect_verdict_omitting_proposed_fix_is_still_a_judge_failure(tmp_path):
    no_fix = verdict(coverage="claim-only", agreement="not-applicable")
    del no_fix["proposed_fix"]
    proc, report = run(tmp_path, envelope(judged()), [no_fix])
    assert "| fr-thing | judge-failed |" in report
    assert "proposed_fix" in report


def test_usefulness_is_aggregated_by_selection_mechanism(tmp_path):
    # The tuning channel: which mechanism produced decisive regions and which
    # produced noise. Without it, threshold changes are taste.
    proc, report = run(tmp_path, envelope(judged()), [verdict()])
    assert "id-anchor" in report
    assert "term-overlap" in report
    assert "decisive" in report


def test_a_verdict_for_an_unknown_neighbourhood_is_an_error(tmp_path):
    stray = verdict(id="requirement/cpt-cf-bss-ledger-fr-nowhere")
    proc, _report = run(tmp_path, envelope(judged()), [stray])
    assert proc.returncode == 1
    assert proc.stderr.startswith("Error: ")


def test_the_report_refuses_to_be_written_inside_a_gear_docs_tree(tmp_path):
    tmp_path.mkdir(parents=True, exist_ok=True)
    nb_path = tmp_path / "n.json"
    v_path = tmp_path / "v.json"
    nb_path.write_text(json.dumps(envelope(judged())), encoding="utf-8")
    v_path.write_text(json.dumps([verdict()]), encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(JUDGE_REPORT_PY),
         "--neighbourhoods", str(nb_path), "--verdicts", str(v_path),
         "--out", "gears/bss/ledger/docs/N1-ledger.md"],
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    assert proc.returncode == 1
    assert proc.stderr.startswith("Error: ")
    assert not (REPO_ROOT / "gears/bss/ledger/docs/N1-ledger.md").exists()


def test_the_report_states_that_judging_was_batched(tmp_path):
    # A deviation the reader cannot see is a deviation nobody can weigh.
    tmp_path.mkdir(parents=True, exist_ok=True)
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps({"batch_size": 4, "batches": [
        {"batch": "batch-01.md", "ids": ["requirement/cpt-cf-bss-ledger-fr-thing",
                                         "requirement/cpt-cf-bss-ledger-fr-other"]},
    ]}), encoding="utf-8")
    nb_path = tmp_path / "n.json"
    v_path = tmp_path / "v.json"
    out = tmp_path / "N1.md"
    nb_path.write_text(json.dumps(envelope(judged())), encoding="utf-8")
    v_path.write_text(json.dumps([verdict()]), encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(JUDGE_REPORT_PY), "--neighbourhoods", str(nb_path),
         "--verdicts", str(v_path), "--batches", str(manifest), "--out", str(out)],
        cwd=str(REPO_ROOT), stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8",
    )
    assert proc.returncode == 0, proc.stderr
    report = out.read_text(encoding="utf-8")
    assert "How this was judged" in report
    assert "not produced in isolation" in report


def test_without_a_manifest_the_report_says_it_cannot_tell(tmp_path):
    _proc, report = run(tmp_path, envelope(judged()), [verdict()])
    assert "cannot say how many dispatches" in report


def test_the_report_is_byte_stable_across_runs(tmp_path):
    first_proc, first = run(tmp_path / "a", envelope(judged()), [verdict()])
    second_proc, second = run(tmp_path / "b", envelope(judged()), [verdict()])
    assert first_proc.returncode == second_proc.returncode == 0
    assert first == second
