from spec_check import regions, requirements
from spec_check.corpus import Corpus


def synthetic(*parts):
    return Corpus.from_parts("synthetic", list(parts))


def numbered(count, word):
    """`count` lines, each carrying `word` and its own line number."""
    return "".join("line {} {}\n".format(n + 1, word) for n in range(count))


#: How many filler documents a selection test pads its corpus with.
#:
#: Not decoration. The document-frequency cutoff is a fraction of the *corpus's*
#: windows, so on a corpus of four windows a term occurring twice scores 50 % and
#: is discarded as non-discriminating — every selection test would then select
#: nothing, having measured nothing. A real corpus is the population the cutoff
#: was designed for: pricing's docs tree yields several hundred windows. Padding
#: to 40 filler windows reproduces that regime without pretending a fixture is a
#: gear.
FILLER_DOCUMENTS = 40


def padded(*parts):
    """A synthetic corpus with a realistic window population.

    Every filler document contributes one window of unique tokens, so no term any
    test cares about has its document frequency inflated by the padding.
    """
    filler = [
        ("filler/{:02d}.md".format(n), "zeta{n} qualia{n} wibble{n} frobnitz{n}\n".format(n=n))
        for n in range(FILLER_DOCUMENTS)
    ]
    return synthetic(*(list(parts) + filler))


def test_windows_are_twelve_lines_stepping_six():
    index = regions.WindowIndex.build(synthetic(("a.md", numbered(24, "alpha"))))
    spans = [(w.file, w.start, w.end) for w in index.windows()]
    assert spans == [
        ("a.md", 1, 12), ("a.md", 7, 18), ("a.md", 13, 24), ("a.md", 19, 24),
    ]


def test_a_short_document_is_one_window():
    index = regions.WindowIndex.build(synthetic(("a.md", numbered(3, "alpha"))))
    assert [(w.start, w.end) for w in index.windows()] == [(1, 3)]


def test_an_empty_document_yields_no_windows():
    assert regions.WindowIndex.build(synthetic(("a.md", ""))).windows() == []


def test_window_terms_use_the_shared_tokeniser():
    index = regions.WindowIndex.build(
        synthetic(("a.md", "The overlay `modelKind` for cpt-cf-bss-x-fr-a is approved.\n"))
    )
    assert index.windows()[0].terms == frozenset({"overlay", "approved"})


def test_document_frequency_drops_terms_over_the_cutoff():
    # `common` appears in every window, `rare` in one. The cutoff is 25 % of the
    # corpus's windows, computed per corpus — `pricing` and `plan` are noise in
    # the pricing corpus and load-bearing elsewhere, so no curated stoplist can
    # do this job.
    corpus = synthetic(
        ("a.md", numbered(12, "common")),
        ("b.md", numbered(12, "common")),
        ("c.md", numbered(12, "common")),
        ("d.md", numbered(11, "common") + "rare token here\n"),
    )
    index = regions.WindowIndex.build(corpus)
    assert index.document_frequency("common") == 1.0
    assert index.document_frequency("rare") == 0.25
    assert index.discriminating(frozenset({"common", "rare"})) == frozenset({"rare"})


def test_a_region_must_carry_enough_of_the_requirements_vocabulary():
    corpus = padded(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
        ("design/01-weak.md", "This slice mentions snapshot overlay only.\n"),
        ("design/02-strong.md",
         "Publish freezes the snapshot, applies the overlay, fixes currency "
         "and rounding.\n"),
    )
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    picked = regions.select(index, req)
    assert [r.file for r in picked] == ["design/02-strong.md"]
    assert picked[0].score >= regions.SCORE_THRESHOLD
    assert picked[0].selected_by == "term-overlap"
    # A fraction of the requirement's six terms, not a count of them. Five match:
    # `freezes` is not `freeze` — there is no stemming, deliberately, since a stem
    # that merges `publish`/`published`/`publisher` also merges words the design set
    # uses to mean different things.
    assert picked[0].score == 0.833
    assert picked[0].matched == 5
    # The weak slice carries 2 of 6 and is not a region at all.
    assert "design/01-weak.md" not in [r.file for r in picked]


def test_the_requirements_own_declaration_is_never_its_own_region():
    corpus = padded(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
    )
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    assert regions.select(index, req) == []


def test_an_id_occurrence_anywhere_is_an_anchor():
    # No traceability convention involved: an id occurrence is an id occurrence,
    # whether the document writes `**Traces to**:`, `**Requirements**:` or nothing.
    corpus = padded(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
        ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-thing`\n\nSome prose.\n"),
        ("design/02-b.md", "(See `cpt-cf-bss-x-fr-thing`.) Different words entirely.\n"),
    )
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    picked = regions.select(index, req)
    assert {r.file for r in picked} == {"design/01-a.md", "design/02-b.md"}
    assert {r.selected_by for r in picked} == {"id-anchor"}


def test_an_anchor_is_kept_below_the_threshold_and_still_scored():
    # An anchor is precise by construction, so the term threshold does not gate
    # it — but it carries its term score anyway, because `covered:strong` requires
    # both facts and they must stay independent.
    corpus = padded(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
        ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-thing`\n\nUnrelated words.\n"),
    )
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    picked = regions.select(index, req)
    assert len(picked) == 1
    assert picked[0].selected_by == "id-anchor"
    assert picked[0].score < regions.SCORE_THRESHOLD


def test_anchors_come_first_and_are_capped_at_four():
    body = ("**Traces to**: `cpt-cf-bss-x-fr-thing`\n\n"
            "Publish freeze snapshot overlay currency rounding.\n")
    parts = [
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
    ]
    for n in range(6):
        parts.append(("design/{:02d}-a.md".format(n + 1), body))
    corpus = padded(*parts)
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    picked = regions.select(index, req)
    assert len([r for r in picked if r.selected_by == "id-anchor"]) == regions.MAX_ANCHORS
    assert [r.selected_by for r in picked][:regions.MAX_ANCHORS] == ["id-anchor"] * 4


def test_at_most_five_regions_so_six_fragments_including_the_declaration():
    body = "Publish freeze snapshot overlay currency rounding tiering approvals.\n"
    parts = [
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding tiering approvals.\n"),
    ]
    for n in range(9):
        parts.append(("design/{:02d}-a.md".format(n + 1), body))
    corpus = padded(*parts)
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    picked = regions.select(index, req)
    assert len(picked) <= regions.MAX_FRAGMENTS - 1
    assert len([r for r in picked if r.selected_by == "term-overlap"]) == regions.MAX_OVERLAP_REGIONS


def test_selection_is_deterministic_and_sorted_by_score_then_path():
    corpus = padded(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze snapshot overlay currency rounding.\n"),
        ("design/01-four.md", "freeze snapshot overlay currency\n"),
        ("design/02-five.md", "freeze snapshot overlay currency rounding\n"),
        ("design/03-four.md", "freeze snapshot overlay currency\n"),
    )
    index = regions.WindowIndex.build(corpus)
    req = requirements.parse(corpus)[0]
    once = [(r.file, r.score) for r in regions.select(index, req)]
    twice = [(r.file, r.score) for r in regions.select(index, req)]
    assert once == twice
    assert once == [
        ("design/02-five.md", 0.833), ("design/01-four.md", 0.667), ("design/03-four.md", 0.667),
    ]
