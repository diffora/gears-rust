import os

import pytest

from conftest import REPO_ROOT
from spec_check.corpus import Corpus, CorpusError, split_lines

PRICING = str(REPO_ROOT / "gears/bss/pricing/docs")


def test_split_lines_matches_rust_str_lines():
    # Rust `str::lines()` splits on "\n", strips a "\r" that preceded it, and does
    # not yield a trailing empty line for text ending in a newline. Python's
    # `splitlines()` would additionally split on \v, \f, \x1c-\x1e, \x85,
    # and   — a divergence that would shift every line number.
    assert split_lines("") == []
    assert split_lines("\n") == [""]
    assert split_lines("a\nb") == ["a", "b"]
    assert split_lines("a\nb\n") == ["a", "b"]
    assert split_lines("a\r\nb\r\n") == ["a", "b"]
    assert split_lines("a\r") == ["a\r"]  # no "\n" follows it, so Rust keeps it
    assert split_lines("a\x0cb") == ["a\x0cb"]  # `splitlines()` would split here


def test_loads_every_markdown_file_under_the_gear_docs_tree():
    corpus = Corpus.load(PRICING)
    # PRD, DESIGN, DECISIONS, STRIPE-GAP-ANALYSIS, 3 ADRs, 13 design/*.md
    assert len(corpus.files()) >= 20
    assert corpus.text("PRD.md") is not None
    assert corpus.text("design/03-price-structure.md") is not None
    assert corpus.text("does-not-exist.md") is None


def test_files_are_returned_in_sorted_path_order():
    # Rust stores them in a BTreeMap, so every consumer that takes "the first
    # matching path" (targets.resolve's S-number and ADR lookups) depends on this.
    paths = [p for p, _ in Corpus.load(PRICING).files()]
    assert paths == sorted(paths)


def test_a_nonexistent_root_is_an_error_not_an_empty_corpus():
    # The failure this guards: a mistyped --gear used to produce an Ok corpus with
    # zero files, so every invariant found nothing and the run exited 0 having
    # checked nothing at all.
    with pytest.raises(CorpusError) as excinfo:
        Corpus.load(PRICING + "/DOES-NOT-EXIST")
    assert "DOES-NOT-EXIST" in str(excinfo.value)


def test_a_file_passed_where_a_directory_is_expected_is_an_error():
    with pytest.raises(CorpusError):
        Corpus.load(PRICING + "/PRD.md")


def test_an_unreadable_subdirectory_is_an_error_not_a_shorter_walk(tmp_path):
    # Rust propagates every WalkDir error (`corpus.rs:33-35`); Python's `os.walk`
    # drops them unless given `onerror`, so a docs subtree it could not read would
    # simply contribute no files and the run would look clean. Same failure class
    # as the missing-root case above, one level down.
    root = tmp_path / "docs"
    root.mkdir()
    (root / "PRD.md").write_text("visible\n", encoding="utf-8")
    locked = root / "design"
    locked.mkdir()
    (locked / "01-foundation.md").write_text("hidden\n", encoding="utf-8")
    os.chmod(str(locked), 0o000)
    try:
        with pytest.raises(CorpusError) as excinfo:
            Corpus.load(str(root))
        assert "walking the docs tree under" in str(excinfo.value)
    finally:
        os.chmod(str(locked), 0o755)


def test_the_root_is_echoed_back_exactly_as_passed(tmp_path):
    # Two findings embed the root verbatim, so it must never be normalised — not
    # made absolute, not resolved, not stripped of a trailing component.
    assert Corpus.from_parts("gears/bss/pricing/docs", []).root() == "gears/bss/pricing/docs"
    (tmp_path / "docs").mkdir()
    relative = str((tmp_path / "docs").relative_to(tmp_path))
    cwd = os.getcwd()
    os.chdir(str(tmp_path))
    try:
        assert Corpus.load(relative).root() == "docs"
    finally:
        os.chdir(cwd)


def _finding_keys(corpus):
    """Every finding the three invariants produce over `corpus`, as comparable
    tuples. Built from the real checkers rather than from a fixture: the claim
    being tested is that a stray document cannot move the *reported* set, which
    is a property of what the invariants see, not of what the loader returns.
    """
    from spec_check.invariants import closure, fr_coverage, propagation
    from spec_check.targets import SeamIndex

    seams = SeamIndex.build([corpus])
    declared = closure.DeclaredInstructions.build([corpus])
    codes = closure.declared_codes_union([corpus])
    findings = (
        propagation.check(corpus, seams, [corpus])
        + fr_coverage.check(corpus)
        + closure.check(corpus, declared, codes)
    )
    return sorted((f.invariant, f.file, f.message) for f in findings)


def _write_small_docs_tree(root):
    """A minimal but genuine gear docs tree, built to be **sensitive** to a stray
    document rather than merely to contain one.

    `BETA_UNPAID` is declared in a Problem-responses block and named by no rule,
    so a clean run reports `P3/code-unreferenced` for it. That is the finding a
    bare mention anywhere else in the corpus discharges — the exact mechanism
    that made a stray report move the live count 7 -> 0, and the same one
    `is_decision_register` had to close for `DECISIONS.md` prose.
    """
    root.mkdir(parents=True, exist_ok=True)
    (root / "PRD.md").write_text(
        "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-alpha`\n\nD-01 governs this.\n",
        encoding="utf-8",
    )
    (root / "DECISIONS.md").write_text(
        "#### D-01 — alpha\n\n**Propagated to**: `PRD.md`\n",
        encoding="utf-8",
    )
    (root / "design").mkdir(exist_ok=True)
    (root / "design" / "01-alpha.md").write_text(
        "**Traces to**: `cpt-cf-bss-x-fr-alpha`\n\n"
        "1. [ ] - `p1` - **Alpha rule:** refuses with `ALPHA_INVALID` - `inst-al-rule`\n\n"
        "**Problem responses (RFC 9457):**\n"
        "- `ALPHA_INVALID` — 422\n"
        "- `BETA_UNPAID` — 409\n",
        encoding="utf-8",
    )


def test_a_report_dropped_into_a_docs_root_changes_nothing_and_is_reported(tmp_path):
    # The recorded incident, reproduced: `load` takes EVERY `*.md` under the root,
    # so a stray document joins the corpus, its prose satisfies P1 citation
    # searches and P3 code references, and a stray *top-level* one also mints a
    # citation stem into P1's vocabulary. A stray report once moved the live
    # finding count 7 -> 0.
    #
    # The stray here is this tool's OWN output shape — `judge_report`'s
    # `docs/spec-check/N1-<gear>.md` — which is the one class the tool can name
    # with certainty, and the exact file `judge_report` already refuses to WRITE
    # into a docs tree. This is the other half of that guard: a copy that arrived
    # by any other route.
    root = tmp_path / "docs"
    _write_small_docs_tree(root)
    before = _finding_keys(Corpus.load(str(root)))
    assert ("P3/code-unreferenced", "design/01-alpha.md",
            "`BETA_UNPAID` is declared in a Problem-responses block but referenced by "
            "no rule") in before, (
        "the fixture must produce the finding a stray mention would pay, or this test "
        "passes over a corpus nothing could have moved"
    )

    # A report that names every id and code in the set — the worst case, because
    # a bare mention is exactly what pays a P1 citation and a P3 reference.
    (root / "N1-x.md").write_text(
        "# N1 report\n\nD-01 was judged specified. `BETA_UNPAID` appears. "
        "`inst-al-rule` appears. `cpt-cf-bss-x-fr-alpha` appears.\n",
        encoding="utf-8",
    )
    (root / "spec-check").mkdir()
    (root / "spec-check" / "N1-x.md").write_text("D-01 again.\n", encoding="utf-8")
    (root / "spec-check" / "notes.md").write_text("D-01 a third time.\n", encoding="utf-8")

    corpus = Corpus.load(str(root))
    assert _finding_keys(corpus) == before, (
        "a document this tool wrote must not change what it reports about the gear"
    )
    assert corpus.text("N1-x.md") is None
    assert corpus.excluded_paths() == [
        "N1-x.md",
        "spec-check/N1-x.md",
        "spec-check/notes.md",
    ], "an exclusion must be disclosed, or it is indistinguishable from a file nobody wrote"


def test_an_ordinary_document_whose_name_merely_contains_the_prefix_is_still_read(tmp_path):
    # The exclusion is a name PREFIX and a directory NAME, not a substring match:
    # a design set is allowed to grow a document called `plan-N1-notes.md`, and a
    # `design/spec-check-alignment.md` is a file, not the excluded directory.
    root = tmp_path / "docs"
    _write_small_docs_tree(root)
    (root / "plan-N1-notes.md").write_text("kept\n", encoding="utf-8")
    (root / "design" / "02-spec-check-alignment.md").write_text("kept\n", encoding="utf-8")

    corpus = Corpus.load(str(root))
    assert corpus.text("plan-N1-notes.md") == "kept\n"
    assert corpus.text("design/02-spec-check-alignment.md") == "kept\n"
    assert corpus.excluded_paths() == []


def test_from_parts_applies_the_same_exclusion_as_load():
    # Or a test could prove a behaviour over a document a real run would never
    # have read — which is the same class of untruth as the stray file itself.
    corpus = Corpus.from_parts(
        "synthetic", [("PRD.md", "kept\n"), ("N1-x.md", "dropped\n")]
    )
    assert corpus.text("PRD.md") == "kept\n"
    assert corpus.text("N1-x.md") is None
    assert corpus.excluded_paths() == ["N1-x.md"]
