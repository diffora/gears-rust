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


def test_resolves_explicit_cross_gear_paths_as_written():
    r = resolve(
        "cross-gear only — `../../rating/docs/DESIGN.md`, "
        "`../../subscriptions/docs/design/05-entitlements.md` (3 sites).",
        pricing(),
        SeamIndex(),
    )
    assert r.paths == [
        "../../rating/docs/DESIGN.md",
        "../../subscriptions/docs/design/05-entitlements.md",
    ]
    assert r.unresolved == []


def test_shorthand_tokens_inside_a_cross_gear_path_are_part_of_that_path():
    # `PRD` and `DESIGN` occur inside the paths; neither may become a second,
    # own-gear claim against pricing's PRD.md / DESIGN.md.
    r = resolve(
        "`../../subscriptions/docs/PRD.md`; `../../rating/docs/DESIGN.md`.",
        pricing(),
        SeamIndex(),
    )
    assert r.paths == [
        "../../rating/docs/DESIGN.md",
        "../../subscriptions/docs/PRD.md",
    ]


def test_a_cross_gear_path_naming_the_citing_gear_folds_to_the_in_corpus_path():
    r = resolve("`../../pricing/docs/PRD.md`", pricing(), SeamIndex())
    assert r.paths == ["PRD.md"]


def test_a_shorthand_token_outside_the_path_still_resolves_own_gear():
    r = resolve("PRD §9.2; also `../../rating/docs/DESIGN.md`", pricing(), SeamIndex())
    assert r.paths == ["../../rating/docs/DESIGN.md", "PRD.md"]


# --- own-gear documents by path and by stem (2026-08-16) -------------------


def synthetic(*extra):
    """A corpus shaped like a real gear: the three documents with a dedicated
    branch, plus whatever the test wants to name.
    """
    parts = [
        ("PRD.md", "Requirements.\n"),
        ("DESIGN.md", "Design digest.\n"),
        ("design/01-foundation.md", "Foundation.\n"),
    ]
    parts.extend(extra)
    return Corpus.from_parts("gears/bss/alpha/docs", parts)


def test_resolves_a_top_level_document_named_by_path():
    # D-319's live form: `` `STRIPE-GAP-ANALYSIS.md` §4 (the row's disposition) ``.
    # Dropped entirely before 2026-08-16 — and *not* reported, because the same
    # citation named PRD, DESIGN and two slices that did resolve.
    corpus = synthetic(("STRIPE-GAP-ANALYSIS.md", "Gaps.\n"))
    r = resolve("PRD §6.1; `STRIPE-GAP-ANALYSIS.md` §4 and G-3.", corpus, SeamIndex())
    assert r.paths == ["PRD.md", "STRIPE-GAP-ANALYSIS.md"]
    assert r.unresolved == []


def test_resolves_a_top_level_document_named_by_stem():
    # D-43's live form: `STRIPE-GAP-ANALYSIS G-2 marked actioned` — the same
    # document, named the way `PRD` and `DESIGN` are named. Those two were never
    # really shorthands; they are the stems of two top-level documents that
    # happened to be hard-coded.
    corpus = synthetic(("STRIPE-GAP-ANALYSIS.md", "Gaps.\n"))
    r = resolve("S1 rows; STRIPE-GAP-ANALYSIS G-2 marked actioned.", corpus, SeamIndex())
    assert r.paths == ["STRIPE-GAP-ANALYSIS.md", "design/01-foundation.md"]


def test_resolves_a_nested_document_named_by_path():
    corpus = synthetic(("design/03-price-structure.md", "Structure.\n"))
    r = resolve("both `design/03-price-structure.md` and ./design/01-foundation.md", corpus,
                SeamIndex())
    assert r.paths == ["design/01-foundation.md", "design/03-price-structure.md"]


def test_a_path_naming_no_document_of_this_corpus_is_reported_not_dropped():
    # The rule that keeps the widening from becoming a new silence. A target
    # shaped like a document of this corpus, naming one the corpus does not
    # hold, is a Finding — never a claim that reads as verified.
    r = resolve("PRD §1; `GONE.md` §2.", synthetic(), SeamIndex())
    assert r.paths == ["PRD.md"]
    assert r.unresolved == ["GONE.md"]


def test_an_unresolvable_path_is_reported_beside_targets_that_did_resolve():
    # The exact shape of the defect being repaired: one citation, one good token
    # and one the resolver cannot map. Reporting only when *nothing* resolves is
    # what let `STRIPE-GAP-ANALYSIS.md` disappear for the life of the tool.
    r = resolve("PRD §1; DESIGN §2; S1 §3; `docs/nowhere.md` §4.", synthetic(), SeamIndex())
    assert r.paths == ["DESIGN.md", "PRD.md", "design/01-foundation.md"]
    assert r.unresolved == ["docs/nowhere.md"]


def test_a_corpus_derived_stem_never_shadows_a_dedicated_shorthand():
    # `PRD`, `DESIGN` and `SEAMS` keep their own branch precisely so a corpus
    # *missing* one of them still reports `unresolved` rather than falling silent
    # — a corpus-derived vocabulary alone would simply stop recognising the word.
    bare = Corpus.from_parts("gears/bss/alpha/docs", [("NOTES.md", "Notes.\n")])
    r = resolve("PRD §1; DESIGN §2.", bare, SeamIndex())
    assert r.paths == []
    assert r.unresolved == ["DESIGN", "PRD"]


def test_a_seams_stem_stays_with_the_seam_branch():
    # `SEAMS.md` is a top-level document of two live gears, so a naive stem
    # vocabulary would add it to `paths` beside — or instead of — the seam
    # branch's own answer, silently changing what a dangling `SEAMS <id>` reports.
    corpus = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [("SEAMS.md", "| # | Seam |\n|---|------|\n| **Z1** | Alpha's. |\n")],
    )
    resolved_known = resolve("SEAMS Z1 note.", corpus, SeamIndex.build([corpus]))
    assert resolved_known.paths == ["SEAMS.md"]
    assert resolved_known.seam_undefined == []

    dangling = resolve("SEAMS Z9 note.", corpus, SeamIndex.build([corpus]))
    assert dangling.paths == []
    assert dangling.seam_undefined == ["Z9"]


def test_a_stem_inside_its_own_path_form_is_not_a_second_claim():
    corpus = synthetic(("STRIPE-GAP-ANALYSIS.md", "Gaps.\n"))
    r = resolve("`STRIPE-GAP-ANALYSIS.md` §4.", corpus, SeamIndex())
    assert r.paths == ["STRIPE-GAP-ANALYSIS.md"]
    assert r.unresolved == []


def test_a_shorthand_inside_a_cross_gear_path_is_still_not_an_own_gear_claim():
    # The pre-existing rule, re-asserted now that a second span-claiming pass
    # runs between the cross-gear pass and the shorthand pass.
    r = resolve("../../beta/docs/PRD.md §2.", synthetic(), SeamIndex())
    assert r.paths == ["../../beta/docs/PRD.md"]
    assert r.unresolved == []


def test_only_top_level_documents_contribute_a_stem():
    # The boundary. A `design/` slice and an `ADR/` are addressed by the
    # shorthands built for them or by explicit path; minting `01-foundation` as a
    # bare word would put file stems nobody writes into the vocabulary.
    corpus = synthetic(("ADR/0001-cpt-cf-bss-alpha-adr-thing.md", "ADR.\n"))
    r = resolve("01-foundation and 0001-cpt-cf-bss-alpha-adr-thing rows.", corpus, SeamIndex())
    assert r.paths == []
    assert r.unresolved == []


def test_the_live_pricing_register_resolves_its_gap_analysis_claims():
    # Both live instances of the class, asserted against the real corpus.
    corpus = pricing()
    seams = known_seams()
    for raw, want in [
        ("STRIPE-GAP-ANALYSIS G-2 marked actioned.", "STRIPE-GAP-ANALYSIS.md"),
        ("`STRIPE-GAP-ANALYSIS.md` §4 (the row's disposition) and G-3.", "STRIPE-GAP-ANALYSIS.md"),
    ]:
        assert want in resolve(raw, corpus, seams).paths, raw


# --- a shorthand is a citation token, not a path segment (2026-08-16b) -----


def test_a_markdown_link_to_a_cross_gear_document_is_one_target_not_two():
    # D-313's prescribed fix, written in the register's own house style — the
    # register already contains `[rating PRD](../../rating/docs/PRD.md)`. Before
    # this rule the label's `PRD` minted a phantom claim into the *citing* gear's
    # own PRD.md, which cites D-313 nowhere, so the finding the fix was meant to
    # clear stayed exactly where it was: the remedy could not be applied at all.
    r = resolve(
        "`inst-tb-window`'s statement and the schema table; "
        "[rating PRD](../../rating/docs/PRD.md) §Definitions, §Time and §539.",
        pricing(),
        known_seams(),
    )
    assert r.paths == ["../../rating/docs/PRD.md"]
    assert "PRD.md" not in r.paths
    assert r.unresolved == []


def test_a_markdown_link_label_is_part_of_its_target_for_every_shorthand():
    for label in ["rating PRD", "rating DESIGN", "rating SEAMS", "the S7 rows", "Foundation"]:
        raw = "[{}](../../rating/docs/PRD.md)".format(label)
        r = resolve(raw, pricing(), known_seams())
        assert r.paths == ["../../rating/docs/PRD.md"], raw
        assert r.seam_undefined == [], raw


def test_a_markdown_link_to_an_own_gear_document_is_also_one_target():
    # D-68's live shape is `` [`design/03-price-structure.md`](./design/03-price-structure.md) ``,
    # where label and destination name the same file — which cannot tell a link
    # pass that handles own-gear destinations from one that does not. So the
    # label here names a *different* document from the destination: only a rule
    # that claims the whole link gets one target out of it.
    r = resolve("[the PRD row](./design/03-price-structure.md) Traces-to", pricing(), SeamIndex())
    assert r.paths == ["design/03-price-structure.md"]
    assert "PRD.md" not in r.paths


def test_a_link_whose_destination_is_not_a_document_claims_nothing():
    # The bound, and it has to put a shorthand in the label to be worth anything:
    # an anchor or an external URL is not a target, so the link claims no span
    # and its label is read exactly as before.
    for dest in ["#the-anchor", "https://example.invalid/x"]:
        r = resolve("[the PRD glossary](" + dest + ") and S1 §2.", pricing(), SeamIndex())
        assert r.paths == ["PRD.md", "design/01-foundation.md"], dest


def test_a_shorthand_inside_a_hyphenated_compound_is_not_a_citation():
    # Live: D-172 cites "S2 §5 (the third-Foundation-refusal paragraph…)", and
    # `\b` held on both sides of that `Foundation` because `-` is a non-word
    # character. It minted `design/01-foundation.md`. The phantom was invisible
    # only because D-172's own `S1` resolves to the same file — masked by
    # duplication, not by being right.
    r = resolve("S2 §5 (the third-Foundation-refusal paragraph).", pricing(), SeamIndex())
    assert r.paths == ["design/02-plan-definition.md"]


def test_a_shorthand_that_is_the_head_of_a_longer_filename_is_not_a_citation():
    # `PRD-product-catalog-marketplace-…`, which the register does write in prose.
    r = resolve("the source UC PRD-product-catalog-marketplace-202601120119 note.",
                pricing(), SeamIndex())
    assert r.paths == []


def test_a_shorthand_beside_a_slash_still_resolves_in_both_directions():
    # The measured bound on the token rule, and why `/` is not rejected:
    # `DESIGN/README` is a real live citation (D-03) of `DESIGN.md`, and `S7/S11`
    # is the same shape waiting to be written. Rejecting `/` would silently drop
    # both — the very defect class this file is being repaired for.
    assert resolve("DESIGN/README, PRD (12 spots)", pricing(), SeamIndex()).paths == [
        "DESIGN.md", "PRD.md",
    ]
    assert resolve("S7/S11 rows", pricing(), SeamIndex()).paths == [
        "design/07-pricewindow-linkage.md", "design/11-lifecycle.md",
    ]


def test_a_shorthand_ending_a_sentence_still_resolves():
    # A trailing period is punctuation, not an extension: only `.` followed by an
    # alphanumeric (`PRD.md`) ends the token.
    assert resolve("propagated to DESIGN. And to PRD.", pricing(), SeamIndex()).paths == [
        "DESIGN.md", "PRD.md",
    ]


def test_a_corpus_stem_is_not_the_head_of_a_longer_name():
    corpus = Corpus.from_parts(
        "gears/bss/alpha/docs",
        [("PRD.md", "R.\n"), ("REVIEW.md", "Review.\n")],
    )
    assert resolve("REVIEW-2026-08 was archived.", corpus, SeamIndex()).paths == []
    assert resolve("REVIEW F-08-1 → fixed.", corpus, SeamIndex()).paths == ["REVIEW.md"]
