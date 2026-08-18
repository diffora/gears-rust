import json

from spec_check.finding import Finding, Severity
from spec_check.invariants.closure import PINNED_UNREFERENCED_CODES_2026_07_29
from spec_check.invariants.propagation import PINNED_PROPAGATION_GAPS_2026_07_29
from spec_check.report import (
    is_known_debt,
    json_report,
    partition_known_debt,
    render_text,
    render_unreproduced_pins,
    unreproduced_pins,
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
    # `pins_not_reproduced` was appended on 2026-08-18, after `known_debt_tracked_as`
    # and before the optional `known_debt`, which is where a new serde struct field
    # declared in that position would serialise. Appended rather than inserted: the
    # order is the Rust binary's struct order and every existing consumer's key
    # offsets stay where they were.
    keys = list(json_report([], [], True).keys())
    assert keys == [
        "findings",
        "known_debt_suppressed",
        "known_debt_tracked_as",
        "pins_not_reproduced",
        "known_debt",
    ]


def test_a_pin_that_did_not_reproduce_is_reported_rather_than_silently_suppressed():
    # The whole point of the field. `is_known_debt` only ever subtracts, so a pin
    # whose document has been fixed goes on suppressing nothing and the run prints
    # the same summary it always did. Here the run produced NO findings at all, so
    # every pinned entry for the gear is dead — and the envelope says so.
    rows = unreproduced_pins([], "pricing")
    assert len(rows) == len(PINNED_PROPAGATION_GAPS_2026_07_29) + len(
        PINNED_UNREFERENCED_CODES_2026_07_29
    )
    assert render_unreproduced_pins(rows).startswith(
        "\n{} pinned finding(s) did not reproduce".format(len(rows))
    )

    payload = json_report([], [], False, rows)
    assert len(payload["pins_not_reproduced"]) == len(rows)
    assert payload["pins_not_reproduced"][0]["invariant"] == "P1/propagation-missing"


def test_a_pin_from_another_gear_is_not_reported_as_this_gears_dead_pin():
    # Both baselines are pricing-only snapshots and `(id, path)` is not unique
    # across gears — the same reason `is_known_debt` takes a gear. A rating run
    # must not be told that pricing's pins are dead.
    assert unreproduced_pins([], "rating") == []


def test_a_reproduced_pin_is_not_reported_as_dead_and_a_full_house_renders_nothing():
    _gear, ident, path = PINNED_PROPAGATION_GAPS_2026_07_29[0]
    rows = unreproduced_pins([pinned_propagation_finding()], "pricing")
    assert ("P1/propagation-missing", ident, path) not in rows
    # An empty result renders to nothing at all — never a header with no rows
    # under it, which reads as a report of something.
    assert render_unreproduced_pins([]) == ""


def test_non_ascii_survives_serialisation_unescaped():
    # serde_json does not escape non-ASCII; `json.dumps` does unless told not to.
    # Findings contain §, × and …, so the frozen JSON oracle depends on this.
    f = Finding("P1/x", Severity.LOW, "DECISIONS.md", 1, "citation `§15 rows ×5.` …")
    text = json.dumps(json_report([f], [], False), indent=2, ensure_ascii=False)
    assert "§15 rows ×5." in text
    assert "\\u00a7" not in text
