from spec_check import regions, requirements
from spec_check.corpus import Corpus
from spec_check.semantic import neighbourhood


def one_requirement():
    corpus = Corpus.from_parts("synthetic", [
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-thing`\n"
         "\n"
         "Publish **MUST** freeze the snapshot.\n"
         "\n"
         "**Actors**: `cpt-cf-bss-x-actor-admin`\n"),
    ])
    return requirements.parse(corpus)[0]


def anchored():
    return regions.Region("design/03-c.md", 88, 99, "Publishing freezes it.", 0.9,
                          "id-anchor", matched=27)


def heuristic():
    return regions.Region("design/02-b.md", 201, 212, "A different account.", 0.62,
                          "term-overlap", matched=18)


def test_contract_shape():
    nb = neighbourhood.build(one_requirement(), [anchored(), heuristic()],
                             "suspicious:multi-region")
    assert nb["id"] == "requirement/cpt-cf-bss-x-fr-thing"
    assert nb["kind"] == "requirement"
    assert nb["gear"] == ""  # a synthetic single-component root has no gear
    assert nb["triage"] == "suspicious:multi-region"
    assert nb["judge"] is True
    assert nb["unbuildable"] == []
    roles = [f["role"] for f in nb["fragments"]]
    assert roles == ["requirement-declaration", "candidate-region", "candidate-region"]


def test_the_declaration_fragment_carries_real_line_numbers_and_the_whole_block():
    nb = neighbourhood.build(one_requirement(), [anchored()], "suspicious:weak-coverage")
    declaration = nb["fragments"][0]
    assert declaration["file"] == "PRD.md"
    assert declaration["lines"] == [3, 5]
    assert "**MUST** freeze" in declaration["text"]
    assert "**Actors**:" in declaration["text"]  # structured sub-fields kept


def test_region_fragments_carry_selection_provenance_in_the_json():
    nb = neighbourhood.build(one_requirement(), [anchored(), heuristic()],
                             "suspicious:multi-region")
    assert [f.get("selected_by") for f in nb["fragments"][1:]] == ["id-anchor", "term-overlap"]
    assert [f.get("score") for f in nb["fragments"][1:]] == [0.9, 0.62]
    assert [f.get("matched_terms") for f in nb["fragments"][1:]] == [27, 18]


def test_judge_rendering_hides_selected_by_and_score():
    # The D-15 control, in code rather than in a prompt: telling a judge a region
    # was anchored biases it toward accepting that region, and D-15 was validated
    # blind. Revealing it is the one A/B worth running during evaluation.
    nb = neighbourhood.build(one_requirement(), [anchored(), heuristic()],
                             "suspicious:multi-region")
    rendered = neighbourhood.render_for_judge(nb)
    for hidden in ("id-anchor", "term-overlap", "selected_by", "score", "triage",
                   "suspicious:multi-region"):
        assert hidden not in rendered
    assert "design/03-c.md:88-99" in rendered
    assert "design/02-b.md:201-212" in rendered
    assert "Publish **MUST** freeze the snapshot." in rendered


def test_judge_rendering_numbers_regions_so_a_verdict_can_name_them():
    nb = neighbourhood.build(one_requirement(), [anchored(), heuristic()],
                             "suspicious:multi-region")
    rendered = neighbourhood.render_for_judge(nb)
    assert "Region 1" in rendered
    assert "Region 2" in rendered


def test_no_prose_is_unbuildable_with_its_reason():
    corpus = Corpus.from_parts("synthetic", [
        ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-bare`\n\n#### Next\n"),
    ])
    bare = requirements.parse(corpus)[0]
    nb = neighbourhood.build(bare, [], "unbuildable:no-prose")
    assert nb["judge"] is False
    assert nb["unbuildable"] == [neighbourhood.UNBUILDABLE_REASONS["unbuildable:no-prose"]]
    assert nb["fragments"] == []
    # No fragment means no line to cite, so the declaration's own line travels
    # with the neighbourhood — the report names PRD.md:N instead of inventing one.
    assert nb["declaration_line"] == 1


def test_no_region_keeps_the_declaration_and_records_the_reason():
    nb = neighbourhood.build(one_requirement(), [], "no-region")
    assert nb["judge"] is False
    assert nb["unbuildable"] == [neighbourhood.UNBUILDABLE_REASONS["no-region"]]
    assert [f["role"] for f in nb["fragments"]] == ["requirement-declaration"]


def test_covered_strong_is_not_judged_but_keeps_its_fragments():
    nb = neighbourhood.build(one_requirement(), [anchored()], "covered:strong")
    assert nb["judge"] is False
    assert nb["unbuildable"] == []
    assert len(nb["fragments"]) == 2
