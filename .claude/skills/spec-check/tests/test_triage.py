from spec_check import regions, requirements
from spec_check.corpus import Corpus
from spec_check.semantic import triage


def req(prose, ident="cpt-cf-bss-x-fr-thing"):
    corpus = Corpus.from_parts("synthetic", [
        ("PRD.md", "- [ ] `p1` - **ID**: `{}`\n\n{}\n".format(ident, prose)),
    ])
    return requirements.parse(corpus)[0]


def region(score, selected_by, path="design/01-a.md"):
    """`score` is the fraction of the requirement's terms the window carries."""
    return regions.Region(path, 1, 12, "text", score, selected_by, matched=int(score * 10))


def test_no_prose_wins_over_everything():
    corpus = Corpus.from_parts("synthetic", [
        ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-bare`\n\n#### Next\n"),
    ])
    bare = requirements.parse(corpus)[0]
    assert triage.classify(bare, [region(0.9, "id-anchor")]) == "unbuildable:no-prose"


def test_no_region_when_prose_exists_and_nothing_matched():
    assert triage.classify(req("Publish **MUST** freeze."), []) == "no-region"


def test_multi_region_outranks_not_normative():
    # Deliberate: a requirement stated vaguely *and* specified in two places needs
    # the disagreement resolved before the wording is tightened, and the judge sees
    # the vague declaration either way.
    vague = req("The catalog handles overlays somehow.")
    picked = [region(0.9, "id-anchor"), region(0.65, "term-overlap", "design/02-b.md")]
    assert triage.classify(vague, picked) == "suspicious:multi-region"


def test_not_normative_for_a_single_region():
    vague = req("The catalog handles overlays somehow.")
    assert triage.classify(vague, [region(0.9, "id-anchor")]) == "suspicious:not-normative"


def test_weak_coverage_for_a_single_account_below_the_strong_bar():
    normative = req("Publish **MUST** freeze the snapshot.")
    assert triage.classify(normative, [region(0.62, "id-anchor")]) == "suspicious:weak-coverage"


def test_regions_that_are_only_citations_are_not_accounts():
    # `fr-addon-rules` in the live corpus: one anchored region carrying one term of
    # 53. A citation cannot contradict anything, so counting it towards
    # multiplicity is what sent 110 of 116 requirements to a judge.
    normative = req("Publish **MUST** freeze the snapshot.")
    citations = [region(0.019, "id-anchor"), region(0.14, "id-anchor", "design/02-b.md")]
    assert triage.classify(normative, citations) == "anchored:no-account"


def test_one_account_among_citations_is_still_one_account():
    normative = req("Publish **MUST** freeze the snapshot.")
    mixed = [region(0.019, "id-anchor"), region(0.9, "id-anchor", "design/02-b.md")]
    assert triage.classify(normative, mixed) == "covered:strong"


def test_the_new_class_is_deterministic_not_judged():
    assert "anchored:no-account" in triage.CLASSES
    assert "anchored:no-account" not in triage.JUDGED


def test_an_anchored_region_just_under_the_strong_bar_is_weak_not_covered():
    # The shape that passes P2 while saying nothing: the id is named, the prose is
    # not there.
    normative = req("Publish **MUST** freeze the snapshot.")
    assert triage.classify(normative, [region(0.65, "id-anchor")]) == "suspicious:weak-coverage"


def test_a_high_scoring_unanchored_region_is_weak_not_covered():
    normative = req("Publish **MUST** freeze the snapshot.")
    assert triage.classify(normative, [region(0.95, "term-overlap")]) == "suspicious:weak-coverage"


def test_covered_strong_needs_anchor_and_score_and_normative_prose():
    normative = req("Publish **MUST** freeze the snapshot.")
    assert triage.classify(normative, [region(0.75, "id-anchor")]) == "covered:strong"


def test_every_normative_keyword_counts():
    for keyword in ("MUST", "MUST NOT", "SHALL", "SHOULD"):
        assert triage.is_normative("The catalog {} do it.".format(keyword))
    assert triage.is_normative("The catalog **MUST** do it.")
    assert not triage.is_normative("The catalog must probably do it.")
    assert not triage.is_normative("A MUSTard-coloured field.")


def test_judged_classes_are_exactly_the_three_suspicious_ones():
    assert triage.JUDGED == frozenset({
        "suspicious:not-normative", "suspicious:multi-region", "suspicious:weak-coverage",
    })
    assert set(triage.CLASSES) >= triage.JUDGED
