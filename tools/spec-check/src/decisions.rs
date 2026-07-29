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
    // Stop at the next bold label: 8 of 68 real entries continue with `**Amendment**`
    // on the SAME physical line, and a greedy `(.+)` swallows those paragraphs whole.
    let propagated = Regex::new(r"\*\*Propagated\*\*:\s*(.+?)(?:\s*\*\*[A-Z]|$)")
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
"#;

    #[test]
    fn parses_entries_but_not_status_board_rows() {
        let ds = parse(SAMPLE);
        assert_eq!(
            ds.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["D-53", "D-54"]
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
}
