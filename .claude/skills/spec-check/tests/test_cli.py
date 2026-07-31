import json

from conftest import live_args, oracle, run_check
from spec_check.finding import Finding, Severity
from spec_check.invariants.propagation import PINNED_PROPAGATION_GAPS_2026_07_29

import check as cli


def pinned_propagation_finding():
    _gear, ident, path = PINNED_PROPAGATION_GAPS_2026_07_29[0]
    return Finding(
        "P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 1,
        "{id} claims propagation into {path}, but that document never cites {id}".format(
            id=ident, path=path
        ),
    )


# --- oracle 1 -------------------------------------------------------------


def test_text_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*live_args())
    assert stdout == oracle("live-text.txt")
    assert code == 0


def test_json_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*(live_args() + ["--format", "json"]))
    assert stdout == oracle("live-json.json")
    assert code == 0


def test_show_known_debt_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*(live_args() + ["--show-known-debt"]))
    assert stdout == oracle("live-show-known-debt.txt")
    assert code == 0


def test_the_run_reports_seven_live_findings_and_seventy_three_suppressed():
    # The handoff's headline numbers, asserted directly rather than only through
    # the byte diff — so a failure says *what* moved, not just that something did.
    # 15/75 until 2026-07-31: the 2026-07-30 slice-review fix round closed the 8
    # live pricing P1/P2/P3 findings and 2 pinned-debt members (hand-checked,
    # notes beside each pinned list), leaving the cross-gear coverage statements.
    # 7/73 until the 2026-07-31 c-wave pin sweep: the same day's a/b/c review fix
    # rounds paid down 4 more pinned members — D-25/PRD (D-93), D-40/design-10
    # (the b-wave L-2 fix), METER_AMBIGUOUS (D-103), TAXONOMY_VALUE_IN_USE
    # (D-120) — all hand-checked, notes beside each pinned list.
    stdout, _ = run_check(*(live_args() + ["--format", "json"]))
    payload = json.loads(stdout)
    assert len(payload["findings"]) == 7
    assert payload["known_debt_suppressed"] == 69
    assert payload["known_debt_tracked_as"] == "D-69"


# --- gate behaviour -------------------------------------------------------


def test_a_run_whose_only_findings_are_pinned_baseline_entries_does_not_fail():
    assert not cli.is_failing([pinned_propagation_finding()], "pricing", "medium")


def test_a_run_with_one_extra_non_baseline_medium_finding_fails():
    findings = [
        pinned_propagation_finding(),
        Finding("P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 2,
                "D-999 claims propagation into PRD.md, but that document never cites D-999"),
    ]
    assert cli.is_failing(findings, "pricing", "medium")


def test_a_pinned_finding_above_the_gate_still_does_not_fail():
    # The whole point of pinning: severity alone never re-triggers a baseline
    # entry, even against the lowest gate.
    assert not cli.is_failing([pinned_propagation_finding()], "pricing", "low")


def test_a_same_keyed_finding_from_a_different_gear_fails_the_run():
    assert cli.is_failing([pinned_propagation_finding()], "rating", "medium")


def test_a_missing_gear_directory_is_an_error_not_a_vacuous_clean_run():
    stdout, code = run_check(
        "--gear", "gears/bss/pricing/docs/DOES-NOT-EXIST", expect_stderr=True
    )
    assert code != 0
    # 1, not 2: Rust's `Termination for Result` prints `Error: {err:?}` and returns
    # `ExitCode::FAILURE`, and the shipping binary was observed doing exactly that.
    # 2 belongs to the argument parser's own usage errors — in clap and in argparse
    # alike — so a load failure must not be indistinguishable from a typo'd flag.
    assert code == 1
    assert stdout == ""
