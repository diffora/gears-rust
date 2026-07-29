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
