use regex::Regex;

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
    let heading = Regex::new(r"^#### (D-\d+)\b").expect("valid heading regex");
    // Stop at the next *field label*: 8 of 68 real entries continue with
    // `**Amendment …**:` on the SAME physical line, and a greedy `(.+)` swallows those
    // paragraphs whole. The boundary requires the closing `**` to be followed by a colon,
    // because every genuine label in this corpus carries one (`**Where**:`, `**Decision**:`,
    // `**Owed**:`) while citations contain colon-less inline bold (`**Formalized as
    // ADR-0003**` in D-03, `**SUB-P7**` in D-65) that must NOT end the capture.
    let propagated = Regex::new(r"\*\*Propagated\*\*:\s*(.+?)(?:\s*\*\*[A-Z][^*]*\*\*:|$)")
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
"#;

    #[test]
    fn parses_entries_but_not_status_board_rows() {
        let ds = parse(SAMPLE);
        assert_eq!(
            ds.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["D-53", "D-54", "D-60", "D-61"]
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
}
