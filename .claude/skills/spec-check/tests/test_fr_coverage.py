from conftest import REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.finding import Severity
from spec_check.invariants.fr_coverage import _defined_requirements, check, is_nfr


def pricing():
    return Corpus.load(str(REPO_ROOT / "gears/bss/pricing/docs"))


def test_every_pricing_fr_is_claimed_by_a_slice():
    assert [f for f in check(pricing()) if f.invariant == "P2/fr-unclaimed"] == []


def test_pricings_nfrs_are_seen_at_all_and_none_is_yet_traced():
    # Until 2026-08-18 both regexes required the literal `-fr-`, which cannot
    # match `-nfr-` (the preceding character is `n`), so all twelve of these were
    # invisible to an invariant whose description says "every PRD requirement".
    # The count is derived from the corpus, not transcribed: this asserts the
    # arm RUNS and covers the whole set, and it keeps holding as the PRD grows.
    corpus = pricing()
    defined_nfrs = [i for i in _defined_requirements(corpus) if is_nfr(i)]
    assert defined_nfrs, "the pricing PRD declares nfr ids; if it stops, delete this test"

    unclaimed = [f for f in check(corpus) if f.invariant == "P2/nfr-unclaimed"]
    # One row per gear, not one per id: the reasoning is in the module docstring.
    assert len(unclaimed) == 1
    assert unclaimed[0].message.startswith(
        "{n} of {n} non-functional requirement(s)".format(n=len(defined_nfrs))
    )
    for ident in defined_nfrs:
        assert ident in unclaimed[0].message, (
            "the collapsed finding must name every unclaimed id, or it trades "
            "attributability for tidiness"
        )


def test_an_nfr_a_slice_does_trace_leaves_the_unclaimed_set():
    # The arm has to be able to go green, or it is a constant rather than a check.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md",
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-alpha`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-nfr-latency`\n"
             "- [ ] `p1` - **ID**: `cpt-cf-bss-x-nfr-uptime`\n"),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-alpha`, `cpt-cf-bss-x-nfr-latency`\n"),
        ],
    )
    findings = check(corpus)
    assert [f.invariant for f in findings if f.invariant == "P2/fr-unclaimed"] == []
    nfr = [f for f in findings if f.invariant == "P2/nfr-unclaimed"]
    assert len(nfr) == 1
    assert nfr[0].message.startswith("1 of 2 non-functional requirement(s)")
    assert "cpt-cf-bss-x-nfr-uptime" in nfr[0].message
    assert "cpt-cf-bss-x-nfr-latency" not in nfr[0].message


def test_a_fully_traced_nfr_set_reports_nothing_at_all():
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-nfr-latency`\n"),
            ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-nfr-latency`\n"),
        ],
    )
    assert [f for f in check(corpus) if f.invariant == "P2/nfr-unclaimed"] == []


def test_a_slice_tracing_an_nfr_the_prd_never_defines_is_dangling_like_an_fr():
    # `fr-dangling` is Medium and is the one half of P2 that proves a document
    # states something false. Widening the id shape must have carried it too, or
    # the widening only added the polite half.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-alpha`\n"),
            ("design/01-a.md",
             "**Traces to**: `cpt-cf-bss-x-fr-alpha`, `cpt-cf-bss-x-nfr-ghost`\n"),
        ],
    )
    dangling = [f for f in check(corpus) if f.invariant == "P2/fr-dangling"]
    assert len(dangling) == 1
    assert dangling[0].severity == Severity.MEDIUM
    assert "cpt-cf-bss-x-nfr-ghost" in dangling[0].message


def test_an_fr_whose_slug_starts_nfr_is_not_mistaken_for_one():
    # The discriminator is the `-nfr-` SEGMENT, not the substring "nfr": an FR
    # called `...-fr-nfr-budget` is an FR, and reporting it under the collapsed
    # per-gear row would hide a real lost owner among a standing property.
    corpus = Corpus.from_parts(
        "synthetic",
        [
            ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-nfr-budget`\n"),
            ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-other`\n"),
        ],
    )
    findings = check(corpus)
    assert [f.invariant for f in findings if f.invariant == "P2/nfr-unclaimed"] == []
    unclaimed = [f for f in findings if f.invariant == "P2/fr-unclaimed"]
    assert len(unclaimed) == 1
    assert "cpt-cf-bss-x-fr-nfr-budget" in unclaimed[0].message


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


def test_subscriptions_is_fully_claimed_after_the_2026_08_01_conversion():
    # Rating left the convention-unknown assertion on 2026-08-01 (its #22
    # conversion), and subscriptions followed the same day: the wave-3 fix wave
    # (#24h) added `**Traces to**:` blocks to all 8 FR-bearing slices — every
    # one of its 47 PRD FRs (hand-counted from the `**ID**:` rows) must now be
    # claimed by exactly one slice, the convention statement gone, the per-id
    # sweep clean. No live gear reports convention-unknown any more; the
    # negative shape stays covered by the synthetic-corpus tests below.
    corpus = Corpus.load(str(REPO_ROOT / "gears/bss/subscriptions/docs"))
    findings = check(corpus)
    for invariant in ("P2/traceability-convention-unknown", "P2/fr-unclaimed",
                      "P2/fr-multiply-claimed", "P2/fr-unknown-id"):
        assert not [f for f in findings if f.invariant == invariant], invariant


def test_rating_is_fully_claimed_after_the_2026_08_01_conversion():
    # The 2026-08-01 billing-domain wave converted rating to `**Traces to**:`;
    # every one of its 43 PRD FRs must be claimed by exactly one slice — the
    # convention statement must be gone and the per-id sweep must be clean.
    corpus = Corpus.load(str(REPO_ROOT / "gears/bss/rating/docs"))
    findings = check(corpus)
    for invariant in ("P2/traceability-convention-unknown", "P2/fr-unclaimed",
                      "P2/fr-multiply-claimed", "P2/fr-unknown-id"):
        assert not [f for f in findings if f.invariant == invariant], invariant


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
