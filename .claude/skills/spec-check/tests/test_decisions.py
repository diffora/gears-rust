from spec_check.decisions import parse

SAMPLE = """
| D-56 | H | a status-board row, not an entry | **DECIDED** |

#### D-53 [M] Reserved capacity on level rows

- **Where**: design/10.
- **Decision**: capacity only.
- **Propagated**: S10 `inst-rv-level` + §5; S3 cross-ref. **Amendment**: scope narrowed to reserved rows only, 2026-07-28.

#### D-54 [L] Something with no propagation line

- **Decision**: nothing to propagate yet.

#### D-60 [M] Plain citation, no continuation

- **Decision**: plain case; nothing follows the citation on the line.
- **Propagated**: S9 `inst-plain-case` + §7; PRD glossary row.

#### D-61 [H] Citation with inline colon-less bold mid-clause

- **Decision**: something formalized, modeled on D-03's shape.
- **Propagated**: S9 `inst-adr-case` + §7 glossary row. **Formalized as ADR-0009** (`cpt-cf-bss-pricing-adr-example-token`).

#### D-62 [M] Qualified label with a same-line continuation

- **Decision**: mirrors the live register's pre-2026-07-29 D-42 shape.
- **Propagated (normative, 2026-07-28)**: S9 `inst-qualified-case` + §7 glossary row. **Amendment**: scope narrowed, 2026-07-28.

#### D-63 [M] Qualified label, nothing follows the citation

- **Propagated (normative, 2026-07-28)**: S9 `inst-plain-qualified-case` + §7; PRD glossary row.

#### D-64 [L] Qualifier with no parentheses stays unrecognized

- **Propagated pending**: S9 `inst-should-not-be-read` + §7.

| SUB-D-18 | M | a prefixed status-board row, still not an entry | **DECIDED** |

#### SUB-D-19 [M] A gear-prefixed entry, subscriptions' live shape

- **Propagated**: PRD §6.8 FR + §9.2 + AC 5; SEAMS SUB-B1.

#### T-D-01 [M] A second, differently-prefixed gear's shape

- **Propagated**: PRD §6.5.
"""


def _by_id(decisions, ident):
    return next(d for d in decisions if d.id == ident)


def test_parses_entries_but_not_status_board_rows():
    assert [d.id for d in parse(SAMPLE)] == [
        "D-53", "D-54", "D-60", "D-61", "D-62", "D-63", "D-64", "SUB-D-19", "T-D-01",
    ]


def test_parses_a_gear_prefixed_decision_id_and_its_propagated_field():
    # The coverage gap the widened id shape closes: subscriptions' register writes
    # `#### SUB-D-NN`, and a bare `^#### (D-\\d+)\\b` anchor matched none of its 19
    # entries — so 19 populated claims went unchecked while the CLI printed a
    # count that read as a clean verdict. `T-D-01` proves it is a shape, not a
    # hardcoded `SUB-` special case.
    ds = parse(SAMPLE)
    assert _by_id(ds, "SUB-D-19").propagated == "PRD §6.8 FR + §9.2 + AC 5; SEAMS SUB-B1."
    assert _by_id(ds, "T-D-01").propagated == "PRD §6.5."


def test_a_gear_prefixed_status_board_row_is_still_not_an_entry():
    assert not any(d.id == "SUB-D-18" for d in parse(SAMPLE))


def test_captures_the_propagated_line_and_only_that_line():
    ds = parse(SAMPLE)
    assert ds[0].propagated == "S10 `inst-rv-level` + §5; S3 cross-ref."
    assert ds[1].propagated is None


def test_records_the_heading_line_number():
    assert parse(SAMPLE)[0].line == 4


def test_stops_the_propagated_capture_before_a_same_line_amendment_label():
    assert "Amendment" not in parse(SAMPLE)[0].propagated


def test_keeps_the_whole_citation_when_nothing_follows_it():
    assert _by_id(parse(SAMPLE), "D-60").propagated == "S9 `inst-plain-case` + §7; PRD glossary row."


def test_does_not_cut_at_colon_less_inline_bold_mid_clause():
    # The right boundary requires the closing `**` to be followed by a colon:
    # every genuine field label in this corpus carries one, while citations
    # contain colon-less inline bold that must NOT end the capture.
    assert _by_id(parse(SAMPLE), "D-61").propagated == (
        "S9 `inst-adr-case` + §7 glossary row. **Formalized as ADR-0009** "
        "(`cpt-cf-bss-pricing-adr-example-token`)."
    )


def test_parses_a_propagated_label_carrying_a_parenthetical_qualifier():
    assert _by_id(parse(SAMPLE), "D-63").propagated == (
        "S9 `inst-plain-qualified-case` + §7; PRD glossary row."
    )


def test_stops_a_qualified_propagated_capture_before_a_same_line_continuation_label():
    assert _by_id(parse(SAMPLE), "D-62").propagated == "S9 `inst-qualified-case` + §7 glossary row."


def test_a_qualifier_without_parentheses_is_not_recognized():
    # The widening accepts a *parenthetical* qualifier only. Anything else must
    # still come back None, or P1's `propagation-label-unparsed` fallback could
    # never fire again.
    assert _by_id(parse(SAMPLE), "D-64").propagated is None


# --- wrapped citations (2026-08-16) ---------------------------------------

WRAPPED = """
#### D-70 [H] A citation long enough to wrap

- **Decision**: modelled on the live D-313 and D-314 entries.
- **Propagated**: S3 §4 (the orthogonality clause), §5 (the new code),
  `inst-tb-window`'s statement and the schema table; PRD §Definitions
  and §539.

#### D-71 [M] A wrapped citation followed by a sibling list item

- **Propagated**: S7 §5 (the ordering constraint) and Foundation §3.7 (the
  `draft` half of the check constraint).
- **Amended by D-99 (2026-08-16)**: DESIGN §4 is NOT part of the claim above.

#### D-73 [M] A wrapped citation followed by a nested list item

- **Propagated**: S2 §1.1 (the capability) and §5 (the two codes),
  §6 (the table).
  - **A note about the entry**: DESIGN §4 is a sub-bullet, not the citation.

#### D-74 [M] A wrapped citation ended by a blank line

- **Propagated**: S5 §3 step 4
  and §8.

Loose prose naming DESIGN §4, which the field must not reach.

#### D-75 [M] A wrapped citation ended by a heading

- **Propagated**: S6 §3
  (the credit-source column).

##### The finding

DESIGN §4 sits under a heading and is not part of the claim.

#### D-76 [M] A wrapped citation ended by a table

- **Propagated**: S11 §2
  (`inst-sy-payload`).

| # | Doc |
|---|-----|
| 1 | DESIGN §4 |

#### D-77 [M] A label whose citation begins on the next line

- **Propagated**:
  S12 §4 (the state list).
"""


def _wrapped(ident):
    return _by_id(parse(WRAPPED), ident).propagated


def test_a_wrapped_citation_is_read_past_its_first_physical_line():
    # The defect this pins: the field was searched line by line, so only the
    # first line of a wrapped citation was ever resolved and every target below
    # the wrap went unchecked *and* unreported. Live instances: D-313 (four
    # lines) and D-314 (six).
    assert _wrapped("D-70") == (
        "S3 §4 (the orthogonality clause), §5 (the new code), `inst-tb-window`'s "
        "statement and the schema table; PRD §Definitions and §539."
    )


def test_the_wrapped_field_stops_at_a_sibling_list_item():
    # The bound on the widening, and the shape it must not swallow: D-158,
    # D-203, D-252 and D-324 all continue with `- **…**:` bullets that are
    # *about* the decision, not part of its propagation surface.
    assert _wrapped("D-71") == (
        "S7 §5 (the ordering constraint) and Foundation §3.7 (the `draft` half of "
        "the check constraint)."
    )


def test_the_wrapped_field_stops_at_a_nested_list_item():
    # D-319's live shape: an indented `  - **One of those five targets …**`
    # sub-bullet directly under the citation.
    assert _wrapped("D-73") == "S2 §1.1 (the capability) and §5 (the two codes), §6 (the table)."


def test_the_wrapped_field_stops_at_a_blank_line():
    assert _wrapped("D-74") == "S5 §3 step 4 and §8."


def test_the_wrapped_field_stops_at_a_heading():
    # D-313's own entry continues with `##### …` prose two blocks later.
    assert _wrapped("D-75") == "S6 §3 (the credit-source column)."


def test_the_wrapped_field_stops_at_a_table_row():
    assert _wrapped("D-76") == "S11 §2 (`inst-sy-payload`)."


def test_a_citation_beginning_on_the_line_below_its_label_is_read():
    # Free with the block rebuild, and worth pinning: the old parser required
    # the citation to start on the label's own physical line, so this shape
    # produced `propagated is None` — indistinguishable from "nothing to
    # propagate".
    assert _wrapped("D-77") == "S12 §4 (the state list)."


def test_an_unwrapped_field_is_returned_byte_for_byte():
    # The regression that matters: every entry in the live corpus but two is one
    # physical line, and rebuilding the block must not touch any of them.
    assert _by_id(parse(SAMPLE), "D-60").propagated == "S9 `inst-plain-case` + §7; PRD glossary row."


def test_the_live_registers_carry_exactly_two_wrapped_citations():
    # Measured, not assumed. Both cite their load-bearing target on line one, so
    # neither was *wrong* before the fix — which is exactly why it was worth
    # fixing before it bit.
    from conftest import LIVE_GEARS, REPO_ROOT
    from spec_check.corpus import Corpus, split_lines

    wrapped = []
    for gear in LIVE_GEARS:
        corpus = Corpus.load(str(REPO_ROOT / gear))
        register = corpus.text("DECISIONS.md")
        if register is None:
            continue
        lines = split_lines(register)
        for d in parse(register):
            if d.propagated is None:
                continue
            if not any(d.propagated in line for line in lines):
                wrapped.append((corpus.root(), d.id))
    assert [ident for _root, ident in wrapped] == ["D-313", "D-314"]
