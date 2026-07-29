from conftest import REPO_ROOT
from spec_check.corpus import Corpus
from spec_check.targets import SeamIndex, _normalize, gear_name, resolve, text_at


def load_gear(name):
    """Loads a real gear's docs corpus by name.

    Test-only: naming a gear here names a real corpus on disk for the test to
    load — it is not resolution logic branching on a gear's identity, which is
    exactly what `SeamIndex` exists to avoid in production code.
    """
    return Corpus.load(str(REPO_ROOT / "gears/bss" / name / "docs"))


def pricing():
    return load_gear("pricing")


def known_seams():
    """The seam index the real CLI builds from all three live BSS gears."""
    return SeamIndex.build([load_gear("pricing"), load_gear("rating"), load_gear("subscriptions")])


def test_resolves_slice_numbers_to_design_files():
    r = resolve("S7 (§1, W1, names)", pricing(), SeamIndex())
    assert r.paths == ["design/07-pricewindow-linkage.md"]
    assert r.unresolved == []


def test_resolves_named_documents():
    r = resolve("PRD §17.4; DESIGN §2; Foundation §4.3", pricing(), SeamIndex())
    assert r.paths == ["DESIGN.md", "PRD.md", "design/01-foundation.md"]


def test_resolves_adrs_by_number():
    r = resolve("ADR-0002 (new) + ADR-0001 amendment note", pricing(), SeamIndex())
    assert r.paths == [
        "ADR/0001-cpt-cf-bss-pricing-adr-canonical-scope-key.md",
        "ADR/0002-cpt-cf-bss-pricing-adr-grandfathering-cohort-axis.md",
    ]


def test_seams_needs_a_seam_id_to_pick_a_gear():
    r = resolve("SEAMS M12 asserts the block", pricing(), known_seams())
    assert r.paths == ["../../rating/docs/SEAMS.md"]

    bare = resolve("SEAMS", pricing(), known_seams())
    assert bare.paths == []
    assert bare.unresolved == ["SEAMS"]


def test_deduplicates_and_sorts():
    r = resolve("PRD §1; PRD §2; S3", pricing(), SeamIndex())
    assert r.paths == ["PRD.md", "design/03-price-structure.md"]


def test_seam_id_survives_markdown_bold_wrapping():
    # Real corpus text (D-65's Propagated field): the seam id is wrapped in bold
    # by the author's markdown. A resolver that requires the id to start right
    # after the whitespace following SEAMS misses this and reports a plainly
    # resolvable citation as unresolved.
    r = resolve("subscriptions SEAMS **SUB-P7**.", pricing(), known_seams())
    assert r.paths == ["../../subscriptions/docs/SEAMS.md"]
    assert r.unresolved == []


def test_each_seams_occurrence_resolves_to_its_own_gear():
    # A whole-string search for "the first SEAMS+id" would collapse this to one
    # gear, silently dropping the other. Each lookup is anchored at the slice
    # following its own occurrence.
    r = resolve(
        "rating SEAMS M9 adopted; subscriptions SEAMS **SUB-P7** owed",
        pricing(),
        known_seams(),
    )
    assert r.paths == ["../../rating/docs/SEAMS.md", "../../subscriptions/docs/SEAMS.md"]
    assert r.unresolved == []


def test_bare_seams_is_not_rescued_by_a_later_unrelated_id():
    r = resolve(
        "SEAMS unspecified; rating SEAMS M12 confirms the same block",
        pricing(),
        known_seams(),
    )
    assert r.paths == ["../../rating/docs/SEAMS.md"]
    assert r.unresolved == ["SEAMS"]


def test_seam_citation_within_the_defining_gear_resolves_to_a_same_corpus_path():
    # subscriptions' own DECISIONS.md cites its own SUB-R1 row this exact way. The
    # resolved path must stay in-corpus rather than escaping via `../../` back to
    # the very corpus it started in.
    r = resolve("SEAMS SUB-R1 note", load_gear("subscriptions"), known_seams())
    assert r.paths == ["SEAMS.md"]
    assert r.unresolved == []
    assert r.seam_undefined == []


def test_seam_id_defined_by_no_loaded_gear_is_reported_as_undefined():
    # "Z9" is shaped like a seam id (so this is not a syntactic miss) but is not a
    # row any real SEAMS.md defines — a defect in the citing document, not a
    # resolver gap.
    r = resolve("SEAMS Z9 pending", pricing(), known_seams())
    assert r.paths == []
    assert r.unresolved == []
    assert r.seam_undefined == ["Z9"]


def test_seam_id_defined_by_two_gears_is_reported_as_a_conflict():
    # No two real gears currently claim the same seam id, so this constructs the
    # conflict directly.
    alpha = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [("SEAMS.md", "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n"
                      "| **Z1** | HIGH | Joint | Alpha's definition. |\n")],
    )
    beta = Corpus.from_parts(
        "gears/bss/beta/docs",
        [("SEAMS.md", "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n"
                      "| **Z1** | HIGH | Joint | Beta's conflicting definition. |\n")],
    )
    r = resolve("rating SEAMS Z1 update", pricing(), SeamIndex.build([alpha, beta]))
    assert r.paths == []
    assert r.seam_conflicts == [("Z1", ["alpha", "beta"])]


def test_gear_name_is_the_docs_parent_directory():
    assert gear_name(Corpus.from_parts("gears/bss/pricing/docs", [])) == "pricing"
    # A bare, single-component synthetic root has no gear: callers treat that
    # corpus as contributing nothing, never as an error.
    assert gear_name(Corpus.from_parts("synthetic", [])) is None


def test_normalize_reproduces_pathbuf_pop_semantics():
    # Not in the port plan: added because a `"/".join(components)` translation
    # renders an absolute path as `//a/c` while Rust's `PathBuf` renders `/a/c`.
    # Inert today (`text_at` only ever compares two paths this function produced,
    # so the extra separator cancels), but the docstring promises lexical
    # normalization and a later caller would be entitled to believe it.
    assert _normalize("a/b/../c") == "a/c"
    assert _normalize("gears/bss/beta/docs/../../alpha/docs/SEAMS.md") == "gears/bss/alpha/docs/SEAMS.md"
    assert _normalize("/a/b/../c") == "/a/c"
    assert _normalize("./a") == "a"
    # `pop()` removes whatever the last component is, including another `..`, and
    # fails only on a root-only or empty buffer — where Rust then pushes `..`.
    assert _normalize("../..") == ""
    assert _normalize("..") == ".."
    assert _normalize("/..") == "/.."
    assert _normalize("/") == "/"


def test_text_at_finds_a_cross_gear_target_in_a_sibling_corpus():
    beta = Corpus.from_parts("gears/bss/beta/docs", [("DECISIONS.md", "body\n")])
    alpha = Corpus.from_parts("gears/bss/alpha/docs", [("SEAMS.md", "alpha seams\n")])
    assert text_at(beta, "../../alpha/docs/SEAMS.md", [beta, alpha]) == "alpha seams\n"
    # In-corpus paths take the fast path and never consult `loaded`, so a
    # single-gear run behaves exactly as a plain `corpus.text(rel)` would.
    assert text_at(beta, "DECISIONS.md", []) == "body\n"
    # A target no loaded corpus provides is None, which the caller must *report*
    # rather than skip.
    assert text_at(beta, "../../alpha/docs/SEAMS.md", [beta]) is None
