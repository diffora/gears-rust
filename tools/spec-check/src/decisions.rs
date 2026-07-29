use regex::Regex;

/// The decision-id shape a `####` register entry heading may carry: `D-NN`, optionally
/// carrying one or more uppercase gear prefixes (`SUB-D-01`, and a future `T-D-01`).
///
/// A convention, so it belongs in code — but it must stay *shape*, never a list of gear
/// names: `(?:[A-Z][A-Z0-9]*-)*` accepts any prefix a sibling gear might mint without this
/// crate learning which gears exist. Pricing's register writes the bare `D-NN` form and
/// subscriptions' writes `SUB-D-NN`; before this shape was widened the parser matched only
/// the bare form, so all 19 of subscriptions' populated `**Propagated**:` claims were
/// never checked and the run reported clean.
pub const DECISION_ID: &str = r"(?:[A-Z][A-Z0-9]*-)*D-\d+";

/// One `#### D-NN …` entry of a gear's `DECISIONS.md`.
#[derive(Debug, Clone)]
pub struct Decision {
    pub id: String,
    /// 1-based line of the `####` heading.
    pub line: usize,
    /// The `**Propagated**:` line, whitespace-collapsed, without the label.
    pub propagated: Option<String>,
}

/// Parses decision entries. Status-board table rows mentioning `D-NN` are not
/// entries and are deliberately not matched — only `####` headings are.
pub fn parse(text: &str) -> Vec<Decision> {
    let heading = Regex::new(&format!(r"^#### ({DECISION_ID})\b")).expect("valid heading regex");
    // Stop at the next *field label*: 8 of 68 real entries continue with
    // `**Amendment …**:` on the SAME physical line, and a greedy `(.+)` swallows those
    // paragraphs whole. The boundary requires the closing `**` to be followed by a colon,
    // because every genuine label in this corpus carries one (`**Where**:`, `**Decision**:`,
    // `**Owed**:`) while citations contain colon-less inline bold (`**Formalized as
    // ADR-0003**` in D-03, `**SUB-P7**` in D-65) that must NOT end the capture.
    //
    // The left anchor accepts an optional parenthetical qualifier inside the bold span
    // itself — `**Propagated (normative, 2026-07-28)**:` as well as the plain
    // `**Propagated**:` — because a qualified label is still the same field, only
    // annotated. `[^*)]*` cannot cross a `*` or a `)`, so the qualifier can never run
    // past the closing `**` of its own bold span (and stops at the first `)`, so it
    // can't swallow a second parenthetical either). A qualifier written any other way
    // (no parens, e.g. `**Propagated pending**:`) is deliberately left unmatched:
    // `propagated` comes back `None` for it exactly as for "nothing to propagate", and
    // P1's `unparsed_propagated_label` fallback (propagation.rs) is what turns that
    // specific shape into a Finding instead of a silent skip.
    let propagated =
        Regex::new(r"\*\*Propagated(?:\s*\([^*)]*\))?\*\*:\s*(.+?)(?:\s*\*\*[A-Z][^*]*\*\*:|$)")
            .expect("valid propagated regex");

    let lines: Vec<&str> = text.lines().collect();
    let starts: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| heading.captures(l).map(|c| (i, c[1].to_string())))
        .collect();

    starts
        .iter()
        .enumerate()
        .map(|(n, (start, id))| {
            let end = starts.get(n + 1).map(|(s, _)| *s).unwrap_or(lines.len());
            let body = &lines[*start..end];
            // The label and its citation sit on one physical line in this corpus, but
            // later prose (`**Amendment**: …`) may follow on that same line — the regex
            // above cuts the capture at that boundary.
            let prop = body
                .iter()
                .find_map(|l| propagated.captures(l))
                .map(|c| c[1].trim().to_string());
            Decision {
                id: id.clone(),
                line: start + 1,
                propagated: prop,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
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
"#;

    #[test]
    fn parses_entries_but_not_status_board_rows() {
        let ds = parse(SAMPLE);
        assert_eq!(
            ds.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            [
                "D-53", "D-54", "D-60", "D-61", "D-62", "D-63", "D-64", "SUB-D-19", "T-D-01"
            ]
        );
    }

    #[test]
    fn parses_a_gear_prefixed_decision_id_and_its_propagated_field() {
        // The critical coverage gap this widening closes: subscriptions' register writes
        // `#### SUB-D-NN`, and the old `^#### (D-\d+)\b` anchor matched none of its 19
        // entries — so 19 populated `**Propagated**:` claims were never checked while the
        // CLI printed a finding count that read as a clean verdict. `T-D-01` proves the
        // shape is a shape, not a hardcoded `SUB-` special case.
        let ds = parse(SAMPLE);
        let sub = ds
            .iter()
            .find(|d| d.id == "SUB-D-19")
            .expect("a gear-prefixed entry must parse");
        assert_eq!(
            sub.propagated.as_deref(),
            Some("PRD §6.8 FR + §9.2 + AC 5; SEAMS SUB-B1.")
        );
        let t = ds
            .iter()
            .find(|d| d.id == "T-D-01")
            .expect("a differently-prefixed entry must parse");
        assert_eq!(t.propagated.as_deref(), Some("PRD §6.5."));
    }

    #[test]
    fn a_gear_prefixed_status_board_row_is_still_not_an_entry() {
        // The widened id must not also widen *what counts as an entry*: only `####`
        // headings do, so the `| SUB-D-18 | … |` status-board row above must stay unparsed
        // exactly as the bare-`D-56` row does.
        let ds = parse(SAMPLE);
        assert!(
            !ds.iter().any(|d| d.id == "SUB-D-18"),
            "a table row is not a decision entry: {:#?}",
            ds.iter().map(|d| &d.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn captures_the_propagated_line_and_only_that_line() {
        let ds = parse(SAMPLE);
        let d53 = &ds[0];
        assert_eq!(
            d53.propagated.as_deref(),
            Some("S10 `inst-rv-level` + §5; S3 cross-ref.")
        );
        assert_eq!(ds[1].propagated, None);
    }

    #[test]
    fn records_the_heading_line_number() {
        let ds = parse(SAMPLE);
        assert_eq!(ds[0].line, 4);
    }

    #[test]
    fn stops_the_propagated_capture_before_a_same_line_amendment_label() {
        let ds = parse(SAMPLE);
        let propagated = ds[0]
            .propagated
            .as_deref()
            .expect("D-53 has a propagated citation");
        assert!(
            !propagated.contains("Amendment"),
            "captured amendment prose into propagated: {propagated:?}"
        );
    }

    #[test]
    fn keeps_the_whole_citation_when_nothing_follows_it() {
        let ds = parse(SAMPLE);
        let d60 = &ds[2];
        assert_eq!(d60.id, "D-60");
        assert_eq!(
            d60.propagated.as_deref(),
            Some("S9 `inst-plain-case` + §7; PRD glossary row.")
        );
    }

    #[test]
    fn does_not_cut_at_colon_less_inline_bold_mid_clause() {
        let ds = parse(SAMPLE);
        let d61 = &ds[3];
        assert_eq!(d61.id, "D-61");
        let propagated = d61
            .propagated
            .as_deref()
            .expect("D-61 has a propagated citation");
        // The whole clause must survive, including the token past the colon-less bold span.
        assert_eq!(
            propagated,
            "S9 `inst-adr-case` + §7 glossary row. **Formalized as ADR-0009** (`cpt-cf-bss-pricing-adr-example-token`)."
        );
        assert!(propagated.contains("cpt-cf-bss-pricing-adr-example-token"));
    }

    #[test]
    fn parses_a_propagated_label_carrying_a_parenthetical_qualifier() {
        // Mirrors the live register's pre-2026-07-29 D-42 shape: a genuine
        // propagation surface recorded under `**Propagated (normative,
        // 2026-07-28)**:` rather than the plain `**Propagated**:` every other
        // entry uses. The anchor must read the citation exactly as it would
        // for the plain form.
        let ds = parse(SAMPLE);
        let d63 = ds.iter().find(|d| d.id == "D-63").expect("D-63 present");
        assert_eq!(
            d63.propagated.as_deref(),
            Some("S9 `inst-plain-qualified-case` + §7; PRD glossary row.")
        );
    }

    #[test]
    fn stops_a_qualified_propagated_capture_before_a_same_line_continuation_label() {
        // The qualifier must not disturb the existing right-boundary behaviour:
        // a qualified label that continues into `**Amendment**:` on the same
        // physical line still cuts the capture at that boundary.
        let ds = parse(SAMPLE);
        let d62 = ds.iter().find(|d| d.id == "D-62").expect("D-62 present");
        let propagated = d62
            .propagated
            .as_deref()
            .expect("D-62 has a propagated citation");
        assert_eq!(propagated, "S9 `inst-qualified-case` + §7 glossary row.");
        assert!(
            !propagated.contains("Amendment"),
            "captured amendment prose into propagated: {propagated:?}"
        );
    }

    #[test]
    fn a_qualifier_without_parentheses_is_not_recognized() {
        // The widened anchor accepts a *parenthetical* qualifier only. A
        // qualifier written any other way must still come back `None` — the
        // widening must not swallow every "Propagated ...**:"-shaped label,
        // or P1/propagation-label-unparsed (propagation.rs) could never fire
        // again.
        let ds = parse(SAMPLE);
        let d64 = ds.iter().find(|d| d.id == "D-64").expect("D-64 present");
        assert_eq!(d64.propagated, None);
    }
}
