from conftest import LIVE_GEARS, REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.finding import Finding, Severity
from spec_check.invariants.closure import (
    PINNED_UNREFERENCED_CODES_2026_07_29,
    DeclaredInstructions,
    check,
    is_design_slice,
    is_pinned_baseline,
    unreferenced_pair,
)


def pricing():
    return Corpus.load(str(REPO_ROOT / "gears/bss/pricing/docs"))


def load_gears(names):
    return [Corpus.load(str(REPO_ROOT / "gears/bss" / n / "docs")) for n in names]


def test_is_design_slice_requires_a_numeric_prefix_under_design():
    assert is_design_slice("design/01-foundation.md")
    assert is_design_slice("design/12-operator-efficiency.md")
    assert not is_design_slice("design/README.md")
    assert not is_design_slice("PRD.md")
    assert not is_design_slice("ADR/0001-example.md")


def test_flags_an_instruction_id_referenced_but_never_declared():
    corpus = Corpus.from_parts(
        "synthetic",
        [("design/01-a.md", "1. Do the thing per `inst-xx-ghost` - `inst-xx-real`\n")],
    )
    findings = check(corpus, DeclaredInstructions.build([corpus]))
    assert len(findings) == 1
    assert findings[0].invariant == "P3/inst-dangling"
    assert "inst-xx-ghost" in findings[0].message


def test_does_not_flag_an_instruction_id_declared_in_a_different_loaded_corpus():
    # An id pricing declares, cited from rating's SEAMS.md without a local
    # re-declaration. Resolving declarations across every loaded corpus — not just
    # the one being checked — is what makes this a legitimate reference.
    corpora = [
        Corpus.from_parts("gears/bss/alpha/docs",
                          [("design/01-a.md", "1. Some rule - `inst-shared-id`\n")]),
        Corpus.from_parts("gears/bss/beta/docs",
                          [("SEAMS.md", "Cites the joint contract `inst-shared-id` here.\n")]),
    ]
    findings = check(corpora[1], DeclaredInstructions.build(corpora))
    assert not [f for f in findings if f.invariant == "P3/inst-dangling"]


def test_flags_an_instruction_id_declared_in_no_loaded_corpus():
    # The cross-corpus union must not become a blanket amnesty for genuinely
    # invented ids.
    corpora = [
        Corpus.from_parts("gears/bss/alpha/docs", [("design/01-a.md", "nothing here\n")]),
        Corpus.from_parts("gears/bss/beta/docs",
                          [("SEAMS.md", "Cites the never-declared `inst-ghost` here.\n")]),
    ]
    dangling = [f for f in check(corpora[1], DeclaredInstructions.build(corpora))
                if f.invariant == "P3/inst-dangling"]
    assert len(dangling) == 1
    assert "inst-ghost" in dangling[0].message


def test_cross_gear_instruction_references_are_not_flagged_against_the_live_corpus():
    # Real-corpus regression: checking rating/subscriptions against only their own
    # declarations produced 8 false positives for ids pricing declares and they
    # legitimately cite. With the declared set built across all three, none.
    corpora = load_gears(["pricing", "rating", "subscriptions"])
    declared = DeclaredInstructions.build(corpora)
    dangling = [f for c in corpora for f in check(c, declared)
                if f.invariant == "P3/inst-dangling"]
    assert dangling == []


def test_pricing_corpus_has_no_dangling_instruction_ids():
    corpus = pricing()
    dangling = [f for f in check(corpus, DeclaredInstructions.build([corpus]))
                if f.invariant == "P3/inst-dangling"]
    assert dangling == []


def test_flags_an_error_code_declared_but_never_referenced():
    corpus = Corpus.from_parts(
        "synthetic",
        [("design/01-a.md",
          "**Problem responses (RFC 9457):** `USED_CODE` (422),\n`ORPHAN_CODE` (409)\n\n"
          "Rule: fails with `USED_CODE`.\n")],
    )
    findings = check(corpus, DeclaredInstructions.build([corpus]))
    assert len(findings) == 1
    assert findings[0].invariant == "P3/code-unreferenced"
    assert "ORPHAN_CODE" in findings[0].message


def test_flags_a_document_that_declares_codes_outside_any_problem_responses_block():
    # Mirrors design/01-foundation.md's shape. Two codes, not one — that makes
    # "exactly one divergence finding" a real assertion about cardinality (one per
    # document) rather than one a bug emitting one per code would also pass. The
    # path is deliberately slice-shaped, or is_design_slice would make this pass
    # vacuously.
    corpus = Corpus.from_parts(
        "synthetic",
        [("design/01-a.md",
          "Foundation-owned failure modes, referenced (never redefined) by slices: "
          "`FIRST_CODE` (409), `SECOND_CODE` (422).\n")],
    )
    divergences = [f for f in check(corpus, DeclaredInstructions.build([corpus]))
                   if f.invariant == "P3/code-convention-divergent"]
    assert len(divergences) == 1
    assert divergences[0].severity == Severity.LOW
    assert divergences[0].file == "design/01-a.md"


def test_does_not_flag_a_non_slice_document_that_references_codes():
    # PRD.md, DECISIONS.md and the ADRs reference codes a slice owns without
    # declaring any — they were never meant to use the convention.
    corpus = Corpus.from_parts(
        "synthetic",
        [("PRD.md", "The publish check fails with `SOME_CODE` when the row is invalid.\n")],
    )
    findings = check(corpus, DeclaredInstructions.build([corpus]))
    assert not [f for f in findings if f.invariant == "P3/code-convention-divergent"]


# --- oracle 3 -------------------------------------------------------------


def test_code_unreferenced_findings_match_the_pinned_2026_07_29_baseline():
    # NOT a green invariant, deliberately. This makes debt visible and stable, not
    # clean — asserting emptiness would hide exactly the kind of gap P3 catches.
    corpus = pricing()
    actual = {
        pair
        for pair in (unreferenced_pair(f) for f in check(corpus, DeclaredInstructions.build([corpus])))
        if pair is not None
    }
    assert all(gear == "pricing" for gear, _, _ in PINNED_UNREFERENCED_CODES_2026_07_29), (
        "this baseline is documented as a pricing-only snapshot; a non-pricing entry "
        "would invalidate this test's (code, file)-only comparison"
    )
    expected = {(code, path) for _, code, path in PINNED_UNREFERENCED_CODES_2026_07_29}

    appeared = sorted(actual - expected)
    disappeared = sorted(expected - actual)
    assert not appeared and not disappeared, (
        "code-unreferenced baseline drifted from the pinned 2026-07-29 set — "
        "newly appeared (not in the pin): {}; no longer reproduced (pin needs "
        "updating — did someone fix these?): {}".format(appeared, disappeared)
    )


def test_the_pinned_code_baseline_has_exactly_fifty_entries():
    # 51 until 2026-07-31 (PACKAGE_FIELDS_INVALID removed); 50 until the same
    # day's c-wave pin sweep (METER_AMBIGUOUS — D-103's rule reference;
    # TAXONOMY_VALUE_IN_USE — D-120's rule reference); notes beside the list.
    assert len(PINNED_UNREFERENCED_CODES_2026_07_29) == 48


def test_is_pinned_baseline_matches_only_the_recorded_gear():
    # `design/03-...`-shaped filenames are just as plausible in a sibling gear's
    # own design set, so a same-keyed finding from another gear must not match.
    gear, code, file = PINNED_UNREFERENCED_CODES_2026_07_29[0]
    assert gear == "pricing"
    finding = Finding(
        "P3/code-unreferenced", Severity.LOW, file, None,
        "`{}` is declared in a Problem-responses block but referenced by no rule".format(code),
    )
    assert is_pinned_baseline(finding, "pricing")
    assert not is_pinned_baseline(finding, "rating")
    assert not is_pinned_baseline(finding, "subscriptions")


def test_unreferenced_pair_ignores_other_invariants():
    assert unreferenced_pair(Finding("P1/propagation-missing", Severity.MEDIUM, "f", None, "m")) is None
