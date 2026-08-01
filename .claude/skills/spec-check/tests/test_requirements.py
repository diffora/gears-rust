from conftest import REPO_ROOT
from spec_check import requirements
from spec_check.corpus import Corpus


def synthetic(*parts):
    return Corpus.from_parts("synthetic", list(parts))


def test_parses_a_declaration_and_its_prose():
    corpus = synthetic(
        ("PRD.md",
         "#### Per-seat pricing\n"
         "\n"
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-per-seat`\n"
         "\n"
         "A row **MUST** persist a unit price.\n"
         "\n"
         "**Rationale**: quantity provenance.\n"
         "\n"
         "#### Next thing\n"),
    )
    reqs = requirements.parse(corpus)
    assert len(reqs) == 1
    req = reqs[0]
    assert req.id == "cpt-cf-bss-x-fr-per-seat"
    assert req.kind == "fr"
    assert req.priority == "p1"
    assert req.file == "PRD.md"
    assert req.line == 3
    assert req.prose_lines == (5, 7)
    assert req.prose == "A row **MUST** persist a unit price.\n\n**Rationale**: quantity provenance."


def test_nfr_is_a_requirement_and_is_not_filed_as_fr():
    # The trap: a greedy gear segment in `cpt-cf-[a-z0-9-]+-(fr|nfr)-` can eat the
    # `n` of `-nfr-` and match `fr`, filing every nfr as an fr. The gear segment is
    # non-greedy so the first `-fr-`/`-nfr-` boundary wins.
    corpus = synthetic(
        ("PRD.md", "- [ ] `p2` - **ID**: `cpt-cf-bss-x-nfr-latency-p99`\n\nBudget **MUST** hold.\n"),
    )
    reqs = requirements.parse(corpus)
    assert [(r.id, r.kind) for r in reqs] == [("cpt-cf-bss-x-nfr-latency-p99", "nfr")]


def test_ignores_declarations_that_are_not_requirements():
    corpus = synthetic(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-usecase-publish`\n"
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-contract-read-model`\n"
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-interface-rest`\n"
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-real`\n"),
    )
    assert [r.id for r in requirements.parse(corpus)] == ["cpt-cf-bss-x-fr-real"]


def test_prose_block_stops_at_the_next_declaration_of_any_kind():
    # Boundaries are found over *all* declarations, then filtered to requirements.
    # The other order lets an fr's prose swallow the following usecase entirely.
    corpus = synthetic(
        ("PRD.md",
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-first`\n"
         "\n"
         "First rule.\n"
         "\n"
         "- [ ] `p1` - **ID**: `cpt-cf-bss-x-usecase-second`\n"
         "\n"
         "Use case prose.\n"),
    )
    reqs = requirements.parse(corpus)
    assert len(reqs) == 1
    assert reqs[0].prose == "First rule."
    assert reqs[0].prose_lines == (3, 3)


def test_prose_block_stops_at_end_of_file():
    corpus = synthetic(
        ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-last`\n\nLast rule.\n"),
    )
    assert requirements.parse(corpus)[0].prose == "Last rule."


def test_an_empty_prose_block_is_recorded_as_such_not_dropped():
    corpus = synthetic(
        ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-bare`\n\n#### Next\n"),
    )
    req = requirements.parse(corpus)[0]
    assert req.prose == ""
    assert req.prose_lines is None


def test_checked_checkbox_is_still_a_declaration():
    corpus = synthetic(
        ("PRD.md", "- [x] `p1` - **ID**: `cpt-cf-bss-x-fr-done`\n\nDone rule.\n"),
    )
    assert [r.id for r in requirements.parse(corpus)] == ["cpt-cf-bss-x-fr-done"]


def test_gear_comes_from_the_corpus_root_not_the_id():
    # The id's own gear segment is `bss-pricing`; the Requirement's gear is
    # `pricing`, exactly as `targets.gear_name` reports it for every other
    # invariant. One notion of gear identity in the tool, not two.
    corpus = Corpus.load(str(REPO_ROOT / "gears/bss/pricing/docs"))
    assert {r.gear for r in requirements.parse(corpus)} == {"pricing"}


def test_terms_are_lowercased_and_at_least_four_characters():
    terms = requirements.derive_terms("The Catalog MUST persist a row per plan.")
    assert "catalog" in terms
    assert "persist" in terms
    for short in ("the", "a", "row", "per"):
        assert short not in terms


def test_terms_drop_function_words():
    terms = requirements.derive_terms("Publish **MUST** validate that each overlay exists.")
    assert "validate" in terms
    assert "overlay" in terms
    for stop in ("must", "that", "each"):
        assert stop not in terms


def test_terms_drop_backticked_content():
    terms = requirements.derive_terms("A `modelKind=per_unit` row **MUST** persist a unit price.")
    assert "persist" in terms
    assert "price" in terms
    assert "modelkind" not in terms
    assert "per_unit" not in terms


def test_terms_drop_whole_ids_before_tokenising():
    # The order is the point. Tokenising first splits
    # `cpt-cf-bss-pricing-fr-per-seat` into `pricing`, `seat`, … — and `pricing`
    # matches nearly every window of the pricing corpus, so the requirement would
    # drag in the whole gear. Ids are anchors; terms are prose.
    terms = requirements.derive_terms(
        "Publishing cpt-cf-bss-pricing-fr-per-seat requires an approved overlay."
    )
    assert "publishing" in terms
    assert "overlay" in terms
    for leaked in ("pricing", "seat"):
        assert leaked not in terms


def test_terms_drop_backticked_ids_too():
    terms = requirements.derive_terms(
        "**Actors**: `cpt-cf-bss-pricing-actor-rating`\n\nThe catalog **MUST** publish."
    )
    assert "catalog" in terms
    assert "rating" not in terms


def test_terms_drop_structured_field_lines():
    # The prose block keeps `**Rationale**:` and `**Actors**:` so the judge sees
    # the whole declaration; term derivation drops those lines, because a
    # rationale explains why the rule exists and is not the rule.
    terms = requirements.derive_terms(
        "Publish **MUST** freeze the snapshot.\n"
        "\n"
        "**Rationale**: auditors demand reproducible invoices.\n"
    )
    assert "freeze" in terms
    assert "snapshot" in terms
    for from_rationale in ("auditors", "demand", "reproducible", "invoices"):
        assert from_rationale not in terms


def test_tokenize_is_the_same_pipeline_for_windows():
    # Windows and requirement prose must be scored against one another, so both
    # sides go through one tokeniser. Asymmetric handling of backticks or ids
    # would make the score meaningless.
    assert requirements.tokenize("A `modelKind` overlay per cpt-cf-bss-x-fr-a") == frozenset(
        {"overlay"}
    )


def pricing():
    return Corpus.load(str(REPO_ROOT / "gears/bss/pricing/docs"))


def ledger():
    return Corpus.load(str(REPO_ROOT / "gears/bss/ledger/docs"))


#: Hand-verified 2026-07-30 against the PRDs, not taken from this parser's first
#: output. pricing PRD: 92 declarations = 65 fr + 11 nfr + 8 usecase + 5 contract
#: + 3 interface. ledger PRD: 48 = 35 + 5 + 3 + 3 + 2. A PRD edit that drops or
#: adds a declaration must be loud, not silent.
#: pricing nfr 11 → 12 (2026-08-01, d-wave billing-domain review): the PRD gained
#: `cpt-cf-bss-pricing-nfr-observability` (§7.1) — a deliberate declaration
#: addition (C-5: ~two dozen slice-declared alarms, several Critical and
#: money-facing, with no PRD requirement obliging routing/runbooks), not drift.
PINNED_COUNTS_2026_07_30 = {
    "pricing": {"fr": 65, "nfr": 12, "total": 77},
    "ledger": {"fr": 35, "nfr": 5, "total": 40},
}


def _counts(corpus):
    reqs = requirements.parse(corpus)
    return {
        "fr": len([r for r in reqs if r.kind == "fr"]),
        "nfr": len([r for r in reqs if r.kind == "nfr"]),
        "total": len(reqs),
    }


def test_pricing_requirement_counts_are_pinned():
    assert _counts(pricing()) == PINNED_COUNTS_2026_07_30["pricing"]


def test_ledger_requirement_counts_are_pinned():
    assert _counts(ledger()) == PINNED_COUNTS_2026_07_30["ledger"]


def test_every_requirement_is_declared_in_the_prd():
    # Not a convention this layer relies on — `parse` scans the whole tree — but a
    # fact worth watching: if a design slice starts declaring requirements, this
    # reddens and the neighbourhood builder needs a second look at self-exclusion.
    for corpus in (pricing(), ledger()):
        assert {r.file for r in requirements.parse(corpus)} == {"PRD.md"}


def test_a_known_pricing_requirement_parses_whole():
    # Line numbers are pinned deliberately. If a PRD edit moves this block, confirm
    # the block itself is unchanged and then update the numbers — do not relax the
    # assertion.
    reqs = {r.id: r for r in requirements.parse(pricing())}
    req = reqs["cpt-cf-bss-pricing-fr-per-seat"]
    assert (req.file, req.line, req.priority, req.kind) == ("PRD.md", 459, "p1", "fr")
    assert req.prose_lines == (461, 465)
    # Prose head updated 2026-07-31: the PR-review fix qualified the FR to
    # non-usage rows (the usage carve-out had lagged the 2026-07-28 kind matrix).
    assert req.prose.startswith(
        "A **non-usage** `modelKind=per_unit` row **MUST** persist a unit price")
    assert "**Actors**:" in req.prose  # the judge sees the whole declaration
    terms = requirements.derive_terms(req.prose)
    assert {"persist", "quantity", "catalog", "metering"} <= terms
    assert "pricing" not in terms  # the id-strip order, on live prose
