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
    # 7/69 until the 2026-07-31 PR-review checker fixes: the code-convention
    # check stopped misreading rating design/04's sibling-owned code reference
    # as a prose declaration (its one code is block-declared in pricing —
    # cross-corpus union, mirroring DeclaredInstructions), so one live false
    # positive left; the stricter citation regex simultaneously exposed
    # D-14 -> PRD.md (false-resolved through rating's `T-D-14`), fixed by citing
    # D-14 at `fr-audit-completeness` rather than pinning it.
    # 6/69 until the 2026-08-01 d-wave billing-domain review paid down one more
    # pinned member — REGION_SCOPE_DENIED (referenced by the new
    # `inst-rb-preview-scope` rule body) — hand-checked, note beside the pinned
    # list.
    # 6/68 until the same day's RATING billing-domain wave: the #22 Traces-to
    # conversion removed rating's P2/traceability-convention-unknown coverage
    # statement (43 FRs now checked per-id, all single-owner — a live finding
    # legitimately resolved, not suppressed); debt unchanged.
    # 5/68 until the same day's SUBSCRIPTIONS wave-3 fix wave: its #24h closed
    # the last three live subscriptions findings — SUB-D-15's citation re-shaped
    # to the resolver grammar (and slice 08 now cites it), SUB-D-16's bare
    # `SEAMS` target now names **SUB-C1**, and the Traces-to conversion across
    # all 8 FR-bearing slices removed the subscriptions
    # P2/traceability-convention-unknown coverage statement (47 FRs checked
    # per-id, single-owner on first pass); debt unchanged.
    # 2/59 since the 2026-08-03 G4 plan-shape docs wave (D-149…D-154): nine
    # pinned P3/code-unreferenced members paid down at once, because the wave
    # rewrote the very rules that raise eight of them (Slice 2's four algorithms
    # and Slice 4's tax-persist/policy steps) and a code named by its own rule
    # stops being unreferenced. Where the wave's prose had only *mentioned* a
    # code from a neighbouring rule or from the register entry, the raising rule
    # was fixed so the removal is a fix rather than a side effect — the five
    # such corrections and the two codes deliberately left pinned
    # (PLANTIER_MISSING, SETUP_ROW_INVALID) are itemised beside the pinned list.
    # Live findings unchanged at 2 — both rating-side, untouched by this wave.
    # 2/58 since the 2026-08-07 D-237…D-246 wave: one pinned member,
    # BRAND_UNKNOWN / design/04, paid down by D-239 — and it is the first member
    # ever removed by *deleting the declaration* rather than by naming the code
    # in the rule that raises it, which is the correct resolution for a code the
    # design set had stopped wanting. Debt is now 21 propagation gaps + 37
    # unreferenced codes. Note beside the pinned list, and see REGENERATE.md
    # entry 23 for why the same edit struck two further codes that were never
    # pinned (both were named in `inst-tx-brand`'s prose, so P3 saw them
    # referenced) — D-239's own claim that the debt covered "the three" was
    # wrong and has been corrected in the register.
    stdout, _ = run_check(*(live_args() + ["--format", "json"]))
    payload = json.loads(stdout)
    assert len(payload["findings"]) == 2
    # 58 until the 2026-08-08 Slice 10 merge, which paid down one pinned member --
    # `FLOOR_TYPE_MISSING` / design/10 -- by naming the code in `inst-ft-typed`,
    # the rule that would raise it, together with the reason it cannot fire in the
    # two-field floor shape the slice actually built. Debt is now 21 propagation
    # gaps + 36 unreferenced codes. Live findings unchanged at 2, both rating-side.
    assert payload["known_debt_suppressed"] == 57
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
