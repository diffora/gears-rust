from conftest import LIVE_GEARS, REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.decisions import parse as parse_decisions
from spec_check.finding import Finding, Severity
from spec_check.invariants import propagation
from spec_check.invariants.propagation import (
    PINNED_PROPAGATION_GAPS_2026_07_29,
    check,
    is_pinned_baseline,
    missing_pair,
)
from spec_check.targets import SeamIndex, gear_name


def live_corpora():
    """Every live BSS gear's corpus, in the order `make spec-check` passes them —
    pricing first, so `live_corpora()[0]` is the corpus the pinned baseline was
    taken from.

    The whole live set, rather than pricing alone: a cross-gear propagation target
    is verified against the sibling document, so a test that loaded one gear would
    silently check something a real run does not.
    """
    return [Corpus.load(str(REPO_ROOT / gear)) for gear in LIVE_GEARS]


def is_cross_gear(path):
    return path.startswith("../")


# --- oracle 2 -------------------------------------------------------------


def test_propagation_gaps_match_the_pinned_2026_07_29_baseline():
    # NOT a green invariant, deliberately. This exists to make debt visible and
    # stable, not to assert the register is clean — it is not, and asserting
    # emptiness would hide exactly the kind of gap P1 exists to catch.
    #
    # Scoped to *in-corpus* targets, which is all this pin ever could have
    # covered: every entry names a pricing-relative path, taken when a
    # `../../<gear>/docs/…` target was unverifiable by construction.
    corpora = live_corpora()
    seams = SeamIndex.build(corpora)
    actual = {
        pair
        for pair in (missing_pair(f) for f in check(corpora[0], seams, corpora))
        if pair is not None and not is_cross_gear(pair[1])
    }
    assert all(gear == "pricing" for gear, _, _ in PINNED_PROPAGATION_GAPS_2026_07_29), (
        "this baseline is documented as a pricing-only snapshot; a non-pricing entry "
        "would invalidate this test's (id, path)-only comparison"
    )
    expected = {(ident, path) for _, ident, path in PINNED_PROPAGATION_GAPS_2026_07_29}

    appeared = sorted(actual - expected)
    disappeared = sorted(expected - actual)
    assert not appeared and not disappeared, (
        "propagation-gap baseline drifted from the pinned 2026-07-29 set — "
        "newly appeared (not in the pin): {}; no longer reproduced (pin needs "
        "updating — did someone fix these?): {}".format(appeared, disappeared)
    )


def test_the_pinned_baseline_has_exactly_twenty_four_entries():
    # A transcription guard: the set comparison above would also fail on a typo,
    # but a dropped line is easier to read as a count.
    assert len(PINNED_PROPAGATION_GAPS_2026_07_29) == 24


def test_cross_gear_propagation_gaps_match_the_expected_set():
    # Four of pricing's decisions claim a cross-gear surface (D-44 `SEAMS M10`,
    # D-46 `SEAMS RG3`, D-60 `SEAMS M12`, D-65 `SEAMS SUB-P7`); checked for real,
    # all four verify clean, so the expected set is empty. Kept as an exact-set
    # assertion rather than a "no gaps" check so the failure message names
    # whatever appears. A newly appeared gap is unaccepted debt to take to a
    # human; a disappeared one means a docs round closed it — never the reverse.
    corpora = live_corpora()
    seams = SeamIndex.build(corpora)
    actual = {
        pair
        for pair in (missing_pair(f) for f in check(corpora[0], seams, corpora))
        if pair is not None and is_cross_gear(pair[1])
    }
    assert actual == set()


def _subject_id(finding):
    """The decision id a P1 message opens with. Empty for
    `P1/decision-register-unparsed`, whose subject is a whole register.

    Deliberately not keyed on (file, line): every one of these sits in
    DECISIONS.md, so a line number would break the expectation on any unrelated
    edit — drift detection that cries wolf.
    """
    if finding.invariant == "P1/decision-register-unparsed":
        return ""
    first = finding.message.split()[0] if finding.message.split() else ""
    return first.rstrip(":")


def test_every_other_live_p1_finding_class_matches_its_exact_expected_set():
    # Exact set over all three gears, keyed (gear, invariant, subject). That makes
    # this an assertion about the *classes that are empty* too: a seam-undefined,
    # seam-conflict, propagation-label-unparsed or propagation-target-not-loaded
    # appearing anywhere would add a tuple and fail here.
    # propagation-missing is excluded — it is pinned exactly, twice, above.
    corpora = live_corpora()
    seams = SeamIndex.build(corpora)
    actual = set()
    for corpus in corpora:
        gear = gear_name(corpus) or ""
        for f in check(corpus, seams, corpora):
            if f.invariant == "P1/propagation-missing":
                continue
            actual.add((gear, f.invariant, _subject_id(f)))

    expected = {
        ("pricing", "P1/propagation-uninterpretable", "D-49"),
        ("pricing", "P1/propagation-uninterpretable", "D-66"),
        ("rating", "P1/decision-register-unparsed", ""),
        ("subscriptions", "P1/propagation-uninterpretable", "SUB-D-15"),
        ("subscriptions", "P1/propagation-unresolvable", "SUB-D-16"),
    }
    assert sorted(actual - expected) == [] and sorted(expected - actual) == []


# --- behaviour ------------------------------------------------------------


def test_flags_a_target_that_does_not_cite_the_decision():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("DECISIONS.md", "#### D-99 [H] Invented\n\n- **Propagated**: PRD §1.\n"),
            ("PRD.md", "Some requirement text with no citation.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-missing"
    assert findings[0].file == "DECISIONS.md"
    assert "D-99" in findings[0].message and "PRD.md" in findings[0].message


def test_reports_unresolvable_targets_separately_and_at_low_severity():
    corpus = Corpus.from_parts(
        "synthetic", [("DECISIONS.md", "#### D-98 [L] Vague\n\n- **Propagated**: SEAMS.\n")]
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-unresolvable"
    assert findings[0].severity == Severity.LOW


def test_every_propagated_label_shape_the_parser_cannot_read_is_still_reported():
    # The fallback used to share the primary parser's exact blind spot — both
    # required the colon *outside* the bold span — so none of these produced
    # anything. Colon-inside-bold is house style in these documents (267
    # occurrences), so one author writing `**Propagated:**` would have silently
    # reintroduced the D-42 false negative.
    for label, body in [
        ("**Propagated:**", "- **Propagated:** PRD §1.\n"),
        ("**Propagated**", "- **Propagated** — PRD §1.\n"),
        ("**Propagated**  :", "- **Propagated**  : PRD §1.\n"),
        ("**Propagated pending**:", "- **Propagated pending**: PRD §1.\n"),
        ("**Propagated (pending review)**", "- **Propagated (pending review)** PRD §1.\n"),
    ]:
        corpus = Corpus.from_parts(
            "synthetic",
            [
                ("DECISIONS.md", "#### D-92 [M] Something\n\n" + body),
                ("PRD.md", "Some requirement text with no citation.\n"),
            ],
        )
        findings = check(corpus, SeamIndex(), [])
        assert len(findings) == 1, "{}: {!r}".format(label, findings)
        assert findings[0].invariant == "P1/propagation-label-unparsed", label
        assert label in findings[0].message, label


def test_the_loose_fallback_does_not_fire_on_the_neighbouring_propagation_status_label():
    # The bound on that looseness: `**Propagation status:**` is a real, different
    # field label in this corpus. Keying on the literal `Propagated` is what stops
    # a shared stem trading one false negative for a false positive.
    corpus = Corpus.from_parts(
        "synthetic",
        [("DECISIONS.md", "#### D-89 [M] Something\n\n- **Propagation status:** tracked in §15.\n")],
    )
    assert check(corpus, SeamIndex(), []) == []


def test_a_citation_the_resolver_understands_nothing_in_is_reported():
    # `resolve` populates `unresolved` only for tokens it *recognised*, so a
    # citation with no recognised token came back all-empty and every loop pushed
    # nothing — a silent skip. Both shapes are live register text.
    for raw in [
        "§15 rows ×5.",
        "rating ×4 files (6 sites); subscriptions ×2 files (3 sites).",
    ]:
        corpus = Corpus.from_parts(
            "synthetic",
            [("DECISIONS.md", "#### D-88 [M] Something\n\n- **Propagated**: " + raw + "\n")],
        )
        findings = check(corpus, SeamIndex(), [])
        assert len(findings) == 1, raw
        assert findings[0].invariant == "P1/propagation-uninterpretable"
        assert findings[0].severity == Severity.LOW
        assert raw in findings[0].message


def test_an_uninterpretable_finding_is_not_also_raised_for_a_merely_unresolvable_token():
    # Guarding on the whole `Resolved` being empty rather than on `paths` alone is
    # what keeps one defect from being reported twice under two names.
    corpus = Corpus.from_parts(
        "synthetic", [("DECISIONS.md", "#### D-87 [M] Something\n\n- **Propagated**: SEAMS.\n")]
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-unresolvable"


def test_a_register_that_yields_zero_entries_says_so_instead_of_reporting_clean():
    corpus = Corpus.from_parts(
        "gears/bss/delta/docs",
        [("DECISIONS.md", "### Decision 1 — not a `####` entry heading\n\n- **Propagated**: PRD §1.\n")],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/decision-register-unparsed"
    assert findings[0].severity == Severity.LOW
    assert "gears/bss/delta/docs" in findings[0].message
    # Suppression must state its cost. "1 `**Propagated`-shaped field", not a bare
    # `1` — the invariant tag and the fixture both contain digits, so a bare-digit
    # check would pass vacuously.
    assert "1 `**Propagated`-shaped field" in findings[0].message


def test_a_corpus_with_no_decision_register_at_all_stays_silent():
    # "There is no register here" is not the same claim as "there is a register I
    # could not read", and only the second is a coverage gap worth a finding.
    corpus = Corpus.from_parts("gears/bss/delta/docs", [("PRD.md", "Requirements.\n")])
    assert check(corpus, SeamIndex(), []) == []


def test_silent_when_the_entry_has_no_propagated_label_at_all():
    corpus = Corpus.from_parts(
        "synthetic",
        [("DECISIONS.md", "#### D-96 [M] Resolved elsewhere\n\n- **Decision**: RESOLVED by D-03.\n")],
    )
    assert check(corpus, SeamIndex(), []) == []


def test_resolves_a_propagated_label_with_a_parenthetical_qualifier():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("DECISIONS.md",
             "#### D-93 [M] Something\n\n- **Propagated (normative, 2026-07-28)**: PRD §1.\n"),
            ("PRD.md", "Some requirement text with no citation.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-missing"


def test_a_cross_gear_target_is_verified_against_the_sibling_corpus_not_skipped():
    # Two sibling corpora, identical except that one cites the deciding id and one
    # does not — the citing one must be silent and the other flagged, which no
    # skip-based implementation can do.
    register = "#### D-90 [M] Cross-gear claim\n\n- **Propagated**: alpha SEAMS Z1.\n"
    header = "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n"
    for cites, want in [(True, 0), (False, 1)]:
        beta = Corpus.from_parts("gears/bss/beta/docs", [("DECISIONS.md", register)])
        body = "Adopted per D-90." if cites else "Adopted, with no citation at all."
        alpha = Corpus.from_parts(
            "gears/bss/alpha/docs",
            [("SEAMS.md", header + "| **Z1** | HIGH | Joint | " + body + " |\n")],
        )
        loaded = [beta, alpha]
        findings = check(loaded[0], SeamIndex.build(loaded), loaded)
        assert len(findings) == want, "cites={}: {!r}".format(cites, findings)
        if want == 1:
            assert findings[0].invariant == "P1/propagation-missing"
            assert "../../alpha/docs/SEAMS.md" in findings[0].message


def test_a_cross_gear_target_no_loaded_corpus_provides_is_reported_not_skipped():
    # When the owning gear was never passed as --gear the claim genuinely cannot be
    # verified — and saying so is the whole point.
    beta = Corpus.from_parts(
        "gears/bss/beta/docs",
        [("DECISIONS.md", "#### D-91 [M] Cross-gear claim\n\n- **Propagated**: alpha SEAMS Z1.\n")],
    )
    alpha = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [("SEAMS.md", "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n"
                      "| **Z1** | HIGH | Joint | Alpha's definition. |\n")],
    )
    findings = check(beta, SeamIndex.build([alpha]), [beta])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-target-not-loaded"
    assert findings[0].severity == Severity.LOW
    assert "../../alpha/docs/SEAMS.md" in findings[0].message
    assert "D-91" in findings[0].message


def test_flags_a_seam_citation_whose_id_no_loaded_gear_defines():
    corpus = Corpus.from_parts(
        "synthetic",
        [("DECISIONS.md",
          "#### D-95 [M] Dangling seam reference\n\n- **Propagated**: SEAMS Z9 note.\n")],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/seam-undefined"
    # Low, not Medium: the seam-id shape also matches any all-caps word after
    # SEAMS (`SEAMS TBD` yields `TBD`), so this is the one class whose false
    # positive is likely — it must never be what fails the default gate.
    assert findings[0].severity == Severity.LOW


def test_flags_a_seam_citation_whose_id_two_loaded_gears_both_define():
    header = "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n"
    corpus = Corpus.from_parts(
        "synthetic",
        [("DECISIONS.md",
          "#### D-94 [M] Conflicting seam ownership\n\n- **Propagated**: SEAMS Z1 note.\n")],
    )
    alpha = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [("SEAMS.md", header + "| **Z1** | HIGH | Joint | Alpha's definition. |\n")],
    )
    beta = Corpus.from_parts(
        "gears/bss/beta/docs",
        [("SEAMS.md", header + "| **Z1** | HIGH | Joint | Beta's conflicting definition. |\n")],
    )
    loaded = [alpha, beta]
    findings = check(corpus, SeamIndex.build(loaded), loaded)
    assert len(findings) == 1
    assert findings[0].invariant == "P1/seam-conflict"
    assert findings[0].severity == Severity.MEDIUM
    for token in ["D-94", "Z1", "alpha", "beta"]:
        assert token in findings[0].message


def test_each_live_gear_register_is_either_parsed_or_says_it_was_not():
    # The property the whole widening exists to protect, asserted against the live
    # tree. Pricing (`D-NN`) and subscriptions (`SUB-D-NN`) must both parse;
    # rating's is a `T-D-NN` table with no propagation surface at all, so the
    # honest outcome there is the zero-entries finding and nothing else.
    corpora = live_corpora()
    seams = SeamIndex.build(corpora)
    unparsed = []
    for corpus in corpora:
        findings = check(corpus, seams, corpora)
        entries = len(parse_decisions(corpus.text("DECISIONS.md")))
        says_unparsed = any(f.invariant == "P1/decision-register-unparsed" for f in findings)
        assert (entries == 0) == says_unparsed, corpus.root()
        if says_unparsed:
            unparsed.append(corpus.root())
            assert len(findings) == 1
    assert len(unparsed) == 1


def test_is_pinned_baseline_matches_only_the_recorded_gear():
    # A finding whose (id, path) matches a pinned pricing entry byte-for-byte must
    # not be known debt when attributed to a different gear — neither `D-NN` nor
    # the target path is unique across gears.
    gear, ident, path = PINNED_PROPAGATION_GAPS_2026_07_29[0]
    assert gear == "pricing"
    finding = Finding(
        "P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 1,
        "{id} claims propagation into {path}, but that document never cites {id}".format(
            id=ident, path=path
        ),
    )
    assert is_pinned_baseline(finding, "pricing")
    assert not is_pinned_baseline(finding, "rating")
    assert not is_pinned_baseline(finding, "subscriptions")


def test_missing_pair_ignores_other_invariants():
    assert missing_pair(Finding("P3/code-unreferenced", Severity.LOW, "f", None, "m")) is None
