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
from spec_check.targets import SeamIndex, gear_name, resolve


def live_corpora():
    """Every live BSS gear's corpus, in the order the removed `make spec-check`
    target passed them — pricing first, so `live_corpora()[0]` is the corpus the
    pinned baseline was taken from.

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
    #
    # Compared against the pin **plus** `LIVE_UNACCEPTED_GAPS_2026_08_16`, which
    # is empty again as of 2026-08-16b — its one member was closed by fixing the
    # document, not by being pinned. The union is kept rather than collapsed back
    # to the pin alone: that list is the place a *newly surfaced, unaccepted* gap
    # goes, and having it already wired is what stops the next one being dropped
    # into the accepted-debt pin because that was the only slot available.
    for expected_gaps in (PINNED_PROPAGATION_GAPS_2026_07_29, LIVE_UNACCEPTED_GAPS_2026_08_16):
        assert all(gear == "pricing" for gear, _, _ in expected_gaps), (
            "these baselines are documented as pricing-only snapshots; a non-pricing "
            "entry would invalidate this test's (id, path)-only comparison"
        )
    expected = {
        (ident, path)
        for _, ident, path in PINNED_PROPAGATION_GAPS_2026_07_29 + LIVE_UNACCEPTED_GAPS_2026_08_16
    }

    appeared = sorted(actual - expected)
    disappeared = sorted(expected - actual)
    assert not appeared and not disappeared, (
        "propagation-gap baseline drifted from the pinned 2026-07-29 set — "
        "newly appeared (not in the pin): {}; no longer reproduced (pin needs "
        "updating — did someone fix these?): {}".format(appeared, disappeared)
    )


def test_the_pinned_baseline_carries_no_duplicate_and_no_foreign_entry():
    # A transcription guard. It asserted `len(...) == N` against a hand-maintained
    # N under a name saying "twenty three" while N was 20 — and a dropped line
    # already surfaces in the two-directional comparison above as "no longer
    # reproduced", so the count added nothing it could not also get wrong. The
    # history it carried is kept; the literal is gone (2026-08-18).
    # 24 until 2026-07-31 (D-01 -> PRD.md removed); 23 until the same day's c-wave
    # pin sweep (D-25 -> PRD.md, D-40 -> design/10 removed — paid down by the
    # a/b review fix rounds); notes beside the list.
    # 21 until 2026-08-16, when D-330's descope wave closed `D-13 -> PRD.md`. D-13
    # governed the historical import; the PRD's strike record for
    # `fr-historical-import-governance` names D-13 as one of the rules that left with
    # the flow, so the claim verifies. Named rather than glossed: the gap was paid by
    # the PRD finally *citing* the decision, not by the PRD acquiring its content —
    # which is what P1 asks and is the honest reading, the requirement D-13
    # propagated into being the one that was struck. Measured against the stashed
    # pre-wave tree (47 -> 45 suppressed), not inferred.
    # The two properties a set comparison cannot see, both derived:
    dupes = sorted(
        entry
        for entry in set(PINNED_PROPAGATION_GAPS_2026_07_29)
        if PINNED_PROPAGATION_GAPS_2026_07_29.count(entry) > 1
    )
    assert dupes == [], (
        "a duplicated entry is invisible to the set comparison above and silently "
        "overstates the debt: {}".format(dupes)
    )
    gears = {gear for gear, _, _ in PINNED_PROPAGATION_GAPS_2026_07_29}
    assert gears == {"pricing"}, (
        "this baseline is documented as a pricing-only snapshot and `(id, path)` is "
        "not unique across gears: {}".format(sorted(gears))
    )


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

    # Until 2026-07-31 pricing also contributed two `propagation-uninterpretable`
    # tuples (D-49 "§15 rows ×5.", D-66 "rating ×4 files"): D-49's field now names
    # PRD, and D-66 cites its cross-gear targets as explicit
    # `../../<gear>/docs/<file>.md` paths, the form `resolve` learned that day.
    # Until 2026-08-01 subscriptions also contributed SUB-D-15
    # (`propagation-uninterpretable`) and SUB-D-16 (`propagation-unresolvable`):
    # the wave-3 fix wave (#24h) re-shaped both citations to the resolver
    # grammar (`S3 §4.5…` / `SEAMS **SUB-C1**`) and slice 08's registry row now
    # cites SUB-D-15, so both resolve and verify.
    expected = {
        ("rating", "P1/decision-register-unparsed", ""),
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


def test_a_hyphen_prefixed_sibling_id_is_not_a_citation():
    # `\b` alone matched `D-99` inside `T-D-99`/`SUB-D-99` (the hyphen is a
    # non-word character, so the boundary held) — a pricing decision counted as
    # cited by a document naming only a sibling gear's id. Measured live before
    # the 2026-07-31 PR-review fix: pricing D-14's PRD claim false-resolved
    # through the rating id `T-D-14`.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("DECISIONS.md", "#### D-99 [H] Invented\n\n- **Propagated**: PRD §1.\n"),
            ("PRD.md", "This row is distinct from pools (rating T-D-99) entirely.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-missing"


def test_compound_and_possessive_mentions_still_count_as_citations():
    # The fix rejects a word character or hyphen *before* the id only: the
    # trailing boundary is unchanged, so `D-99-pattern` and `D-99's` remain
    # citations and `D-9` still cannot match inside `D-99`.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("DECISIONS.md", "#### D-99 [H] Invented\n\n- **Propagated**: PRD §1.\n"),
            ("PRD.md", "The D-99-pattern guard applies; see D-99's rationale.\n"),
        ],
    )
    assert check(corpus, SeamIndex(), []) == []


def test_reports_unresolvable_targets_separately_and_at_low_severity():
    corpus = Corpus.from_parts(
        "synthetic", [("DECISIONS.md", "#### D-98 [L] Vague\n\n- **Propagated**: SEAMS.\n")]
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1
    assert findings[0].invariant == "P1/propagation-unresolvable"
    assert findings[0].severity == Severity.LOW


def test_flags_a_propagated_label_shape_the_widened_parser_still_cannot_read():
    # `decisions.parse`'s anchor reads a *parenthetical* qualifier
    # (`**Propagated (normative, 2026-07-28)**:`, D-42's shape until it was
    # normalised). A qualifier written any other way must still come back `None`,
    # so `unparsed_propagated_label`'s fallback stays reachable: an unresolvable
    # propagation target must be a Finding, never a silent skip, and this shape
    # was never in scope for the widening.
    #
    # Restored from `propagation.rs:718-742`, which the plan's test file dropped.
    # The table-driven test below covers this label shape, but only asserts the
    # invariant id — the severity and the reported file are pinned nowhere else,
    # and the Rust test that pinned them does not survive the crate's removal.
    corpus = Corpus.from_parts(
        "synthetic",
        [("DECISIONS.md", "#### D-97 [M] Something\n\n- **Propagated pending**: PRD §1.\n")],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1, "unexpected: {!r}".format(findings)
    assert findings[0].invariant == "P1/propagation-label-unparsed"
    assert findings[0].severity == Severity.MEDIUM
    assert findings[0].file == "DECISIONS.md"
    assert "D-97" in findings[0].message
    assert "Propagated pending" in findings[0].message


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


# --- own-gear documents outside the shorthand table (2026-08-16) ----------


#: Live, **unaccepted** propagation gaps: reported by the checker, not
#: suppressed, and failing the default `--max-severity medium` gate.
#:
#: Deliberately NOT merged into `PINNED_PROPAGATION_GAPS_2026_07_29`, which is a
#: snapshot of *accepted* debt taken on one day and whose contents are a human
#: decision — the D-46 precedent beside that list is the rule: a brand-new
#: finding put there would be buried. A gap listed here stays in the CLI's
#: output and keeps failing the run until someone fixes the document.
#:
#: **Empty, and it has been non-empty exactly once.**
#:
#: - `D-313 -> PRD.md`: surfaced 2026-08-16 when the parser learned to read a
#:   citation past its first physical line, and **closed the same day by fixing
#:   the register** rather than by pinning it. D-313's field wraps over four
#:   lines and its `PRD` token sat on line two, inside the clause "rating PRD
#:   §Definitions, §Time and §539" — a *cross-gear* claim written in prose,
#:   which as written named the citing gear's own `PRD.md` (0 citations). It now
#:   reads `` `../../rating/docs/PRD.md` ``, D-66's form and the only precedent
#:   inside a `**Propagated**:` field, and rating's `PRD.md` cites D-313 three
#:   times. The claim is *verified*, not merely quiet — see
#:   `test_d_313_cross_gear_claim_stays_resolvable_and_is_actually_checked`.
#:
#: The list is kept rather than deleted with its member: it is the slot a newly
#: surfaced, unaccepted gap belongs in, and an existing empty slot is what stops
#: the next one being dropped into the accepted-debt pin for want of anywhere
#: else to put it.
LIVE_UNACCEPTED_GAPS_2026_08_16 = ()


def test_the_live_unaccepted_gaps_are_not_also_pinned_as_accepted_debt():
    # The two lists answer different questions and must never overlap: one says
    # "known, tracked, does not fail the run", the other "found, live, fails it".
    assert not set(LIVE_UNACCEPTED_GAPS_2026_08_16) & set(PINNED_PROPAGATION_GAPS_2026_07_29)


def test_a_document_outside_the_shorthand_table_is_checked_by_path():
    # Before 2026-08-16 this produced **nothing**: the target was dropped, and
    # `propagation-uninterpretable` never fired either, because the same
    # citation carried a `PRD` the resolver did recognise. The claim read as
    # verified. Live instances: D-319 and D-43, both into
    # `STRIPE-GAP-ANALYSIS.md`, and SUB-D-19 into subscriptions' `REVIEW.md`.
    corpus = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [
            ("DECISIONS.md",
             "#### D-86 [M] Gap analysis claim\n\n"
             "- **Propagated**: PRD §6.1; `STRIPE-GAP-ANALYSIS.md` §4.\n"),
            ("PRD.md", "Requirement text citing D-86.\n"),
            ("STRIPE-GAP-ANALYSIS.md", "G-4 stays open. No citation here at all.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1, "{!r}".format(findings)
    assert findings[0].invariant == "P1/propagation-missing"
    assert "STRIPE-GAP-ANALYSIS.md" in findings[0].message


def test_a_document_outside_the_shorthand_table_is_checked_by_stem():
    # D-43's live form, which carries no `.md` at all.
    corpus = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [
            ("DECISIONS.md",
             "#### D-85 [M] Gap analysis claim\n\n"
             "- **Propagated**: PRD §17.7; STRIPE-GAP-ANALYSIS G-2 marked actioned.\n"),
            ("PRD.md", "Requirement text citing D-85.\n"),
            ("STRIPE-GAP-ANALYSIS.md", "G-2 is actioned. No citation here at all.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1, "{!r}".format(findings)
    assert findings[0].invariant == "P1/propagation-missing"
    assert "STRIPE-GAP-ANALYSIS.md" in findings[0].message


def test_an_unresolvable_document_target_is_reported_beside_resolvable_siblings():
    # The precise failure mode being repaired, at the invariant's own level: a
    # citation carrying one good token and one the resolver cannot map must
    # report the second. Reporting only when *nothing* resolved is what made a
    # whole class of claim invisible.
    corpus = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [
            ("DECISIONS.md",
             "#### D-84 [M] Claim into a document that is not there\n\n"
             "- **Propagated**: PRD §1; `RETIRED-ANALYSIS.md` §2.\n"),
            ("PRD.md", "Requirement text citing D-84.\n"),
        ],
    )
    findings = check(corpus, SeamIndex(), [])
    assert len(findings) == 1, "{!r}".format(findings)
    assert findings[0].invariant == "P1/propagation-unresolvable"
    assert findings[0].severity == Severity.LOW
    assert "RETIRED-ANALYSIS.md" in findings[0].message


def test_the_previously_unchecked_live_claims_are_now_armed_against_their_targets():
    # A claim that resolves is not yet a claim that is *checked*: three of these
    # verify clean, and a test asserting only "no finding" would pass just as
    # well against the tool that dropped them. So each is armed — the decision
    # id is stripped from the document it claims, and the finding must appear.
    import re

    corpora = live_corpora()
    by_gear = {gear_name(c): c for c in corpora}
    cases = [
        ("pricing", "D-43", "STRIPE-GAP-ANALYSIS.md"),
        ("pricing", "D-319", "STRIPE-GAP-ANALYSIS.md"),
        ("subscriptions", "SUB-D-19", "REVIEW.md"),
    ]
    for gear, ident, document in cases:
        corpus = by_gear[gear]
        seams = SeamIndex.build(corpora)
        base = {f.message for f in check(corpus, seams, corpora)}
        assert not any(
            ident in message and document in message for message in base
        ), "{} -> {} is expected to verify clean today".format(ident, document)

        pattern = r"(?<![A-Za-z0-9-])" + re.escape(ident) + r"\b"
        files = dict(corpus.files())
        assert re.search(pattern, files[document]), (
            "{} must actually be cited in {} for this arming to mean anything".format(
                ident, document
            )
        )
        files[document] = re.sub(pattern, "<stripped>", files[document])
        stripped = list(corpora)
        stripped[corpora.index(corpus)] = Corpus.from_parts(corpus.root(), files)
        armed = {f.message for f in check(stripped[corpora.index(corpus)],
                                          SeamIndex.build(stripped), stripped)}
        appeared = armed - base
        assert appeared == {
            "{id} claims propagation into {doc}, but that document never cites {id}".format(
                id=ident, doc=document
            )
        }, "{} -> {}: {!r}".format(ident, document, sorted(appeared))


def test_d_313_cross_gear_claim_stays_resolvable_and_is_actually_checked():
    # This replaces `test_the_prescribed_fix_for_d_313_actually_clears_its_finding`,
    # whose premise the register edit of 2026-08-16b removed: it patched a broken
    # citation in memory and required the finding to clear, and the citation is no
    # longer broken. Deleting it would have thrown away the only thing pinning
    # what the fix bought, so it became the regression instead — and a stronger
    # one, because "no finding" is exactly what the *pre-fix* tool also reported
    # for this claim. Three things are asserted, in the order they can fail:
    #
    #   1. the citation still resolves to the sibling gear's document, and to no
    #      in-corpus `PRD.md` — the phantom that made the remedy inapplicable;
    #   2. it verifies clean against rating's real corpus today;
    #   3. it is *checked*, proved by stripping the id from rating's PRD and
    #      requiring the finding to appear. Without (3) this test would pass
    #      against a checker that dropped the target entirely, which is the whole
    #      defect class this file exists to guard.
    import re

    corpora = live_corpora()
    pricing, rating = corpora[0], corpora[1]
    assert gear_name(rating) == "rating"

    entry = next(d for d in parse_decisions(pricing.text("DECISIONS.md")) if d.id == "D-313")
    resolved = resolve(entry.propagated, pricing, SeamIndex.build(corpora))
    assert "../../rating/docs/PRD.md" in resolved.paths
    assert "PRD.md" not in resolved.paths, (
        "the cross-gear target must not also mint a claim into pricing's own PRD.md"
    )
    assert resolved.unresolved == []

    base = {f.message for f in check(pricing, SeamIndex.build(corpora), corpora)}
    assert not any("D-313" in message for message in base)

    pattern = r"(?<![A-Za-z0-9-])D-313\b"
    files = dict(rating.files())
    assert len(re.findall(pattern, files["PRD.md"])) == 3
    files["PRD.md"] = re.sub(pattern, "<stripped>", files["PRD.md"])
    armed = [pricing, Corpus.from_parts(rating.root(), files)] + corpora[2:]
    appeared = {f.message for f in check(armed[0], SeamIndex.build(armed), armed)} - base
    assert appeared == {
        "D-313 claims propagation into ../../rating/docs/PRD.md, but that document "
        "never cites D-313"
    }, sorted(appeared)


def test_d_313_cross_gear_claim_is_honest_about_needing_the_sibling_gear():
    # The other half of what the corrected citation buys, and the reason the fix
    # is not a downgrade: run pricing alone and the claim is *reported as
    # unverified* rather than quietly passing against a same-named own-gear
    # document. That is the same answer D-66's cross-gear targets give.
    pricing = live_corpora()[0]
    alone = [f for f in check(pricing, SeamIndex.build([pricing]), [pricing])
             if "D-313" in f.message]
    assert len(alone) == 1
    assert alone[0].invariant == "P1/propagation-target-not-loaded"
    assert "../../rating/docs/PRD.md" in alone[0].message
