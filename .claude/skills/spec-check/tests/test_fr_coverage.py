from conftest import REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.finding import Severity
from spec_check.invariants.fr_coverage import check


def pricing():
    return Corpus.load(str(REPO_ROOT / "gears/bss/pricing/docs"))


def test_every_pricing_fr_is_claimed_by_a_slice():
    assert [f for f in check(pricing()) if f.invariant == "P2/fr-unclaimed"] == []


def test_flags_an_fr_no_slice_traces_to():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md",
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-lonely`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-other`\n"),
            ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-other`\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/fr-unclaimed"
    assert "cpt-cf-bss-x-fr-lonely" in findings[0].message


def test_reads_traces_to_lines_that_wrap():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md",
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-first`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-wrapped`\n"),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-first`,\n`cpt-cf-bss-x-fr-wrapped`\n\n"
             "Next paragraph.\n"),
        ],
    )
    assert check(corpus) == []


def test_flags_a_requirement_two_slices_both_claim():
    # P2's brief is "claimed by exactly one slice", but nothing ever reported
    # len() > 1 — the map has always been id -> *set of files*, so the data was
    # there and the check was missing. The finding must name every claiming slice:
    # "somebody claims this twice" is not actionable, "these two do" is.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-shared`\n"),
            ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-shared`\n"),
            ("design/02-b.md", "**Traces to**: `cpt-cf-bss-x-fr-shared`\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/fr-multiply-claimed"
    assert findings[0].severity == Severity.LOW
    assert "design/01-a.md" in findings[0].message
    assert "design/02-b.md" in findings[0].message


def test_one_slice_claiming_a_requirement_twice_is_not_multiply_claimed():
    # Claims deduplicate per file precisely so a copy/paste or line-wrap
    # repetition inside one block is not two owners. Ownership is per document.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-once`\n"),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-once`, `cpt-cf-bss-x-fr-once`\n"),
        ],
    )
    assert check(corpus) == []


def test_multiply_claimed_pricing_requirements_match_what_the_live_design_set_shows():
    # Not accepted debt and deliberately not in a pinned register: these stay live
    # Low findings for a human to rule on. This only keeps the set stable — one
    # appearing, or one being resolved, must both fail here. The four found on
    # 2026-07-30 were ruled on and resolved 2026-07-31: each FR kept exactly one
    # owning slice (S4 invoice-currency-binding, S1 mutation-idempotency +
    # price-amount-validation, S3 per-seat), the pruned slices carrying a
    # delegation note in place of the claim.
    actual = sorted(
        (
            f.message.split()[0],
            int(f.message.split(" is claimed by ")[1].split()[0]),
        )
        for f in check(pricing())
        if f.invariant == "P2/fr-multiply-claimed"
    )
    assert actual == []


def test_flags_a_slice_tracing_to_a_requirement_that_does_not_exist():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-real`\n"),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-real`, `cpt-cf-bss-x-fr-ghost`\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/fr-dangling"
    assert findings[0].file == "design/01-a.md"


def test_deduplicates_a_dangling_id_repeated_in_one_document():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", ""),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-ghost`, `cpt-cf-bss-x-fr-ghost`\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/fr-dangling"
    assert findings[0].file == "design/01-a.md"


def test_recognises_this_slice_directly_addresses_as_an_alternate_claim_convention():
    # Mirrors design/01-foundation.md's exact shape: marker line, then a *blank*
    # line, then bullets, then end of file with no trailing heading. A fixture
    # whose bullets started immediately after the marker would pass even with the
    # wrong "stop at the first blank line" rule — the blank line is what proves
    # the stop condition is right.
    #
    # Two ids, not one: that makes "exactly one divergence finding" a real
    # assertion about cardinality (one per document) rather than one that would
    # also pass a bug emitting one finding per id.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md",
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-foundational`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-alsofoundational`\n"),
            ("design/01-a.md",
             "This slice directly addresses:\n\n"
             "- `cpt-cf-bss-x-fr-foundational` / `cpt-cf-bss-x-fr-alsofoundational` "
             "— does the thing\n"),
        ],
    )
    findings = check(corpus)
    assert not [f for f in findings if f.invariant == "P2/fr-unclaimed"]
    divergences = [f for f in findings if f.invariant == "P2/traceability-convention-divergent"]
    assert len(divergences) == 1
    assert divergences[0].severity == Severity.LOW
    assert divergences[0].file == "design/01-a.md"


def test_flags_a_gear_using_no_known_traceability_convention_instead_of_per_id_noise():
    # Mirrors rating's and subscriptions' real shape: the PRD defines requirements
    # but no slice uses either known marker — instead a third, unparsed convention.
    # P2 has no way to tell claimed from unclaimed there, so it must say so once
    # rather than emit a finding for every requirement it cannot verify.
    corpus = Corpus.from_parts(
        "gears/bss/gamma/docs",
        [
            ("PRD.md",
             "- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-one`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-two`\n"),
            ("design/01-a.md", "## 5. Traceability\n\n- **PRD**: §6.3 `fr-one`; §6.4 `fr-two`\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/traceability-convention-unknown"
    assert findings[0].severity == Severity.LOW
    # Suppression must state its cost. "2 requirement" rather than a bare '2' —
    # the invariant tag "P2" itself contains a 2, so a bare-digit check would pass
    # vacuously.
    assert "2 requirement" in findings[0].message


def test_rating_and_subscriptions_report_convention_unknown_not_per_id_noise():
    # The exact unchecked count is asserted per gear — 43 for rating, 47 for
    # subscriptions, hand-counted from each PRD's `**ID**:` rows.
    for gear, unchecked in [("rating", 43), ("subscriptions", 47)]:
        corpus = Corpus.load(str(REPO_ROOT / "gears/bss" / gear / "docs"))
        findings = check(corpus)
        unknown = [f for f in findings if f.invariant == "P2/traceability-convention-unknown"]
        assert len(unknown) == 1, gear
        assert not [f for f in findings if f.invariant == "P2/fr-unclaimed"], gear
        assert "{} requirement".format(unchecked) in unknown[0].message, gear


def test_a_traces_to_marker_outside_a_design_slice_does_not_count_as_a_known_convention():
    # A stray `**Traces to**:` in a non-slice document (an ADR quoting the
    # convention as an example) must not count as this gear "using a known
    # convention" — nor may it silently absorb a claim, or a requirement genuinely
    # unclaimed by any real slice would read as claimed by accident.
    corpus = Corpus.from_parts(
        "gears/bss/gamma/docs",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-one`\n"),
            ("ADR/0001-example.md",
             "Design slices should write **Traces to**: `cpt-cf-bss-gamma-fr-one` "
             "per convention.\n"),
        ],
    )
    findings = check(corpus)
    assert len(findings) == 1
    assert findings[0].invariant == "P2/traceability-convention-unknown"
    assert "1 requirement" in findings[0].message
