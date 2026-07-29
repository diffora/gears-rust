import json

from spec_check.finding import Finding, Severity
from spec_check.invariants.closure import PINNED_UNREFERENCED_CODES_2026_07_29
from spec_check.invariants.propagation import PINNED_PROPAGATION_GAPS_2026_07_29
from spec_check.report import (
    is_known_debt,
    json_report,
    partition_known_debt,
    render_text,
)


def pinned_propagation_finding():
    _gear, ident, path = PINNED_PROPAGATION_GAPS_2026_07_29[0]
    return Finding(
        "P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 1,
        "{id} claims propagation into {path}, but that document never cites {id}".format(
            id=ident, path=path
        ),
    )


def non_baseline_finding():
    return Finding(
        "P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 2,
        "D-999 claims propagation into PRD.md, but that document never cites D-999",
    )


def pinned_code_unreferenced_finding():
    _gear, code, file = PINNED_UNREFERENCED_CODES_2026_07_29[0]
    return Finding(
        "P3/code-unreferenced", Severity.LOW, file, None,
        "`{}` is declared in a Problem-responses block but referenced by no rule".format(code),
    )


def test_a_pinned_propagation_gap_is_known_debt():
    assert is_known_debt(pinned_propagation_finding(), "pricing")


def test_a_pinned_code_unreferenced_finding_is_known_debt():
    assert is_known_debt(pinned_code_unreferenced_finding(), "pricing")


def test_an_unpinned_finding_with_the_same_invariant_tag_is_not_known_debt():
    assert not is_known_debt(non_baseline_finding(), "pricing")


def test_a_same_keyed_finding_from_a_different_gear_is_not_known_debt():
    # The baseline is a snapshot of one specific corpus, and neither `D-NN` nor
    # the target path is unique across gears — a byte-identical finding from
    # another gear is new drift there, not pricing's pinned debt.
    assert not is_known_debt(pinned_propagation_finding(), "rating")
    assert not is_known_debt(pinned_code_unreferenced_finding(), "rating")


def test_partition_separates_baseline_entries_from_new_drift():
    live, debt = partition_known_debt(
        [pinned_propagation_finding(), non_baseline_finding()], "pricing"
    )
    assert len(live) == 1 and len(debt) == 1
    assert not is_known_debt(live[0], "pricing")
    assert is_known_debt(debt[0], "pricing")


def test_partition_does_not_suppress_a_same_keyed_finding_attributed_to_another_gear():
    live, debt = partition_known_debt([pinned_propagation_finding()], "rating")
    assert len(live) == 1 and debt == []


def test_partition_preserves_input_order_within_each_group():
    a, b, c = non_baseline_finding(), pinned_propagation_finding(), non_baseline_finding()
    c.line = 3
    live, debt = partition_known_debt([a, b, c], "pricing")
    assert [f.line for f in live] == [2, 3]
    assert [f.line for f in debt] == [1]


def test_the_default_summary_discloses_how_many_findings_were_suppressed():
    # The one line that makes suppression honest rather than silent.
    out = render_text([non_baseline_finding()], [pinned_propagation_finding()], False)
    assert "\n1 finding(s)" in out
    assert "1 known-debt finding(s) suppressed, tracked as D-69" in out
    assert "--show-known-debt" in out
    # Suppressed means suppressed: the withheld finding's own text must not be
    # printed, or the count line would describe something already on screen.
    assert pinned_propagation_finding().message not in out


def test_show_known_debt_renders_the_suppressed_findings_and_switches_the_summary():
    out = render_text([non_baseline_finding()], [pinned_propagation_finding()], True)
    assert "Known debt — accepted, tracked as D-69, not new drift (1 finding(s)):" in out
    assert pinned_propagation_finding().message in out
    assert "1 known-debt finding(s) shown above, tracked as D-69" in out
    assert "suppressed" not in out


def test_a_run_with_no_known_debt_prints_no_known_debt_summary_at_all():
    for show in [False, True]:
        out = render_text([non_baseline_finding()], [], show)
        assert "\n1 finding(s)" in out
        assert "known-debt" not in out
        assert "Known debt" not in out


def test_the_json_envelope_reports_the_suppressed_count_and_withholds_the_findings_by_default():
    live = [non_baseline_finding()]
    debt = [pinned_propagation_finding()]

    default = json_report(live, debt, False)
    assert default["known_debt_suppressed"] == 1
    assert default["known_debt_tracked_as"] == "D-69"
    # Absent, not null.
    assert "known_debt" not in default

    assert "known_debt" in json_report(live, debt, True)


def test_the_json_envelope_key_order_is_serdes_struct_order():
    keys = list(json_report([], [], True).keys())
    assert keys == ["findings", "known_debt_suppressed", "known_debt_tracked_as", "known_debt"]


def test_non_ascii_survives_serialisation_unescaped():
    # serde_json does not escape non-ASCII; `json.dumps` does unless told not to.
    # Findings contain §, × and …, so the frozen JSON oracle depends on this.
    f = Finding("P1/x", Severity.LOW, "DECISIONS.md", 1, "citation `§15 rows ×5.` …")
    text = json.dumps(json_report([f], [], False), indent=2, ensure_ascii=False)
    assert "§15 rows ×5." in text
    assert "\\u00a7" not in text
