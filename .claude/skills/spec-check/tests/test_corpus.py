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
