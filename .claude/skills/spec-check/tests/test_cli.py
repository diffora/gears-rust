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

#: The live run's exit code. **0**, as it was for the whole life of this tool
#: except for one day: entry 25's parser fix surfaced a real Medium finding that
#: was not accepted debt (`D-313 -> PRD.md`) and the run went red, and entry 27's
#: register fix closed it — the citation now names `../../rating/docs/PRD.md`,
#: which rating's PRD cites three times. Red for a real reason, then green
#: because the document was fixed, which is the cycle working. Pinned as a name
#: rather than a literal so that a change here has to be a deliberate edit with a
#: reason beside it.
LIVE_EXIT_CODE = 0


def test_text_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*live_args())
    assert stdout == oracle("live-text.txt")
    assert code == LIVE_EXIT_CODE


def test_json_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*(live_args() + ["--format", "json"]))
    assert stdout == oracle("live-json.json")
    assert code == LIVE_EXIT_CODE


def test_show_known_debt_output_is_byte_identical_to_the_rust_binary():
    stdout, code = run_check(*(live_args() + ["--show-known-debt"]))
    assert stdout == oracle("live-show-known-debt.txt")
    assert code == LIVE_EXIT_CODE


def test_the_live_run_passes_the_default_gate_because_nothing_live_is_above_it():
    # The exit code above is an assertion about *why*, not just about a number.
    # Green because the two live findings are both Low — not because the gate is
    # off, not because a Medium finding was quietly pinned into the accepted-debt
    # list. This is the shape that would have caught `D-313 -> PRD.md` being
    # buried instead of fixed, so it survives the finding it was written for.
    stdout, code = run_check(*(live_args() + ["--format", "json"]))
    payload = json.loads(stdout)
    above_gate = [f for f in payload["findings"] if f["severity"] in ("medium", "high")]
    assert [f["message"] for f in above_gate] == []
    assert {f["severity"] for f in payload["findings"]} == {"low"}
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
    # 2 -> 3 on 2026-08-16, and it was the **checker** that moved, not the
    # documents — the fourth such capture in the tool's life and the first since
    # 2026-07-31. P1 learned to read a `**Propagated**:` field past its first
    # physical line, and D-313's citation wraps over four, so its second line's
    # `PRD` token became visible: as written ("rating PRD §Definitions") the
    # claim named the citing gear's own PRD.md, which cites D-313 nowhere. The
    # same capture made three previously-invisible claims *checkable* and all
    # three verify clean (D-43 and D-319 into STRIPE-GAP-ANALYSIS.md, SUB-D-19
    # into subscriptions' REVIEW.md), so they add no finding here; they are armed
    # in `test_the_previously_unchecked_live_claims_are_now_armed_against_their_targets`.
    #
    # 3 -> 2 the same day, and this time it is the **document**: D-313's clause
    # now reads `` `../../rating/docs/PRD.md` `` — D-66's form, the only
    # precedent inside a propagation field — and rating's PRD cites D-313 three
    # times, so the claim verifies for real. Closed by fixing the register, never
    # by pinning it; `LIVE_UNACCEPTED_GAPS_2026_08_16` is empty again. The count
    # is back where it was before entry 25, and the three oracle files are
    # byte-identical to their pre-entry-25 selves — but the two live findings are
    # now the only two there are, rather than the only two the tool could see.
    stdout, _ = run_check(*(live_args() + ["--format", "json"]))
    payload = json.loads(stdout)
    assert len(payload["findings"]) == 2
    # 58 until the 2026-08-08 Slice 10 merge, which paid down one pinned member --
    # `FLOOR_TYPE_MISSING` / design/10 -- by naming the code in `inst-ft-typed`,
    # the rule that would raise it, together with the reason it cannot fire in the
    # two-field floor shape the slice actually built. Debt is now 21 propagation
    # gaps + 33 unreferenced codes (36 until D-256 paid down the three composite
    # entries). Live findings unchanged at 2, both rating-side.
    # 54 until 2026-08-09, when D-278 paid down `CLONE_SOURCE_NOT_FOUND` /
    # design/12 -- the clone route was built, the code minted, and the raising
    # rule named it (`inst-cl-source`). Named in the RULE and not only in the
    # register: a bare code token in register prose closes this finding too, and
    # closing it that way is a false payment. Checked by removing the register
    # mention and confirming the finding stayed closed. Debt is now 21
    # propagation gaps + 32 unreferenced codes. Live findings unchanged at 2.
    # 53 until 2026-08-09, when D-291 built Phase 2 and `inst-bk-phase2` named
    # `BULK_ROW_CONFLICT` -- the rule that raises it. Named in the RULE and not
    # only in the register: the register mention alone closes this finding too,
    # and that is a false payment. Checked by rewording the mention and
    # confirming the finding stayed closed -- the third time this trap has been
    # sprung, see REGENERATE.md entries 24 and 25.
    # 52 -> 49 on 2026-08-14. Three code-unreferenced members left the set: two
    # paid by D-312 (`EVAL_POLICY_MISPLACED` / design/03 and
    # `RESERVATION_ON_NON_USAGE` / design/10, each now named by the rule that
    # raises it) and one, `RUN_SELECTOR_EMPTY` / design/12, that had already
    # left in an earlier wave and was only surfacing now because the oracles had
    # not been re-captured since D-291. See closure.py for the per-member notes.
    # 47 -> 45 on 2026-08-16, by the D-330 historical-import descope wave, and the
    # two members are two different kinds of payment. `BACKDATE_GRANT_REQUIRED` /
    # design/05 left because its DECLARATION was deleted with the flow whose 403 it
    # named -- the second member ever paid that way, after D-239's BRAND_UNKNOWN.
    # `D-13 -> PRD.md` left the propagation pin because the PRD's strike record for
    # `fr-historical-import-governance` names D-13 as one of the rules that went with
    # the flow, so a claim that had never verified now does. Debt is 20 propagation
    # gaps + 25 unreferenced codes. Live findings unchanged at 2, both rating-side.
    # 45 -> 50 on 2026-08-17, one document movement and one checker change, and the
    # arithmetic separates them: 45 -> 44 when this branch's D-343 commits wrote
    # `PHASE_GRAPH_INVALID` into the register's prose and it left the debt set by false
    # payment; then 44 -> 50 when `is_decision_register` stopped register prose paying,
    # returning that member (+1) and revealing five that had been paid the same way all
    # along (+5): COMPOSITE_CONSTITUENT_UNPUBLISHED, GRANDFATHERED_ROW_IMMUTABLE,
    # GRANDFATHER_LOOSEN_FORBIDDEN, MIGRATION_ALREADY_EFFECTIVE, ROUNDING_POLICY_UNKNOWN.
    assert payload["known_debt_suppressed"] == 50
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
