use std::collections::BTreeSet;

use regex::Regex;

use crate::Corpus;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub paths: Vec<String>,
    pub unresolved: Vec<String>,
}

/// Maps the register's propagation shorthand onto corpus-relative paths.
///
/// Cross-gear targets are returned as `../../<gear>/docs/<file>` — relative to the
/// corpus root — so a finding names something a reader can open.
pub fn resolve(raw: &str, corpus: &Corpus) -> Resolved {
    let token = Regex::new(r"\b(S(\d{1,2})|Foundation|PRD|DESIGN|SEAMS|ADR-(\d{4}))\b")
        .expect("valid token regex");
    // `\**` tolerates markdown bold around the seam id — the real citation in
    // DECISIONS.md (D-65) reads "subscriptions SEAMS **SUB-P7**.", and without it
    // that seam id is invisible to this regex, so the token falls back to bare
    // SEAMS and misreports a resolvable citation as unresolved.
    //
    // Anchored at the start of the slice that follows THIS `SEAMS` occurrence (see
    // call site below), so a clause citing two sibling gears resolves each `SEAMS`
    // to its own id instead of both — via an unanchored, whole-string search —
    // collapsing onto whichever id happens to appear first.
    let seam_gear =
        Regex::new(r"^\s+(?:§\w+\s+)?\**(M\d+|RG\d+|SUB-[A-Za-z0-9]+)").expect("valid seam regex");

    let mut paths = BTreeSet::new();
    let mut unresolved = BTreeSet::new();

    for cap in token.captures_iter(raw) {
        let whole_match = cap.get(1).expect("group 1 always matches");
        let whole = whole_match.as_str().to_string();
        match whole.as_str() {
            "PRD" => insert_if_present(corpus, "PRD.md", &mut paths, &mut unresolved, &whole),
            "DESIGN" => insert_if_present(corpus, "DESIGN.md", &mut paths, &mut unresolved, &whole),
            "Foundation" => insert_if_present(
                corpus,
                "design/01-foundation.md",
                &mut paths,
                &mut unresolved,
                &whole,
            ),
            "SEAMS" => match seam_gear.captures(&raw[whole_match.end()..]) {
                Some(c) if c[1].starts_with("SUB-") => {
                    paths.insert("../../subscriptions/docs/SEAMS.md".to_string());
                }
                Some(_) => {
                    paths.insert("../../rating/docs/SEAMS.md".to_string());
                }
                None => {
                    unresolved.insert(whole);
                }
            },
            _ => {
                if let Some(n) = cap.get(2) {
                    let want = format!("design/{:02}-", n.as_str().parse::<u32>().unwrap_or(0));
                    match corpus
                        .files()
                        .map(|(p, _)| p)
                        .find(|p| p.starts_with(&want))
                    {
                        Some(p) => {
                            paths.insert(p.to_string());
                        }
                        None => {
                            unresolved.insert(whole);
                        }
                    }
                } else if let Some(n) = cap.get(3) {
                    let want = format!("ADR/{}-", n.as_str());
                    match corpus
                        .files()
                        .map(|(p, _)| p)
                        .find(|p| p.starts_with(&want))
                    {
                        Some(p) => {
                            paths.insert(p.to_string());
                        }
                        None => {
                            unresolved.insert(whole);
                        }
                    }
                }
            }
        }
    }

    Resolved {
        paths: paths.into_iter().collect(),
        unresolved: unresolved.into_iter().collect(),
    }
}

fn insert_if_present(
    corpus: &Corpus,
    rel: &str,
    paths: &mut BTreeSet<String>,
    unresolved: &mut BTreeSet<String>,
    token: &str,
) {
    if corpus.has(rel) {
        paths.insert(rel.to_string());
    } else {
        unresolved.insert(token.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Corpus;
    use std::path::PathBuf;

    fn pricing() -> Corpus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gears/bss/pricing/docs");
        Corpus::load(&root).expect("pricing corpus loads")
    }

    #[test]
    fn resolves_slice_numbers_to_design_files() {
        let r = resolve("S7 (§1, W1, names)", &pricing());
        assert_eq!(r.paths, ["design/07-pricewindow-linkage.md"]);
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn resolves_named_documents() {
        let r = resolve("PRD §17.4; DESIGN §2; Foundation §4.3", &pricing());
        assert_eq!(r.paths, ["DESIGN.md", "PRD.md", "design/01-foundation.md"]);
    }

    #[test]
    fn resolves_adrs_by_number() {
        let r = resolve("ADR-0002 (new) + ADR-0001 amendment note", &pricing());
        assert_eq!(
            r.paths,
            [
                "ADR/0001-cpt-cf-bss-pricing-adr-canonical-scope-key.md",
                "ADR/0002-cpt-cf-bss-pricing-adr-grandfathering-cohort-axis.md",
            ]
        );
    }

    #[test]
    fn seams_needs_a_seam_id_to_pick_a_gear() {
        let r = resolve("SEAMS M12 asserts the block", &pricing());
        assert_eq!(r.paths, ["../../rating/docs/SEAMS.md"]);

        let bare = resolve("SEAMS", &pricing());
        assert!(bare.paths.is_empty());
        assert_eq!(bare.unresolved, ["SEAMS"]);
    }

    #[test]
    fn deduplicates_and_sorts() {
        let r = resolve("PRD §1; PRD §2; S3", &pricing());
        assert_eq!(r.paths, ["PRD.md", "design/03-price-structure.md"]);
    }

    #[test]
    fn seam_id_survives_markdown_bold_wrapping() {
        // Real corpus text (DECISIONS.md D-65's Propagated field): the seam id is
        // wrapped in `**...**` by the author's markdown — "subscriptions SEAMS
        // **SUB-P7**." A resolver that requires the id to start right after the
        // whitespace following SEAMS (or its optional section prefix) misses this
        // and reports the whole citation as a bare, unresolved SEAMS, even though
        // it plainly names the subscriptions seam.
        let r = resolve("subscriptions SEAMS **SUB-P7**.", &pricing());
        assert_eq!(r.paths, ["../../subscriptions/docs/SEAMS.md"]);
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn each_seams_occurrence_resolves_to_its_own_gear() {
        // The register already propagates one decision (D-66) jointly to both
        // sibling gears; it just hasn't cited both via the `SEAMS` shorthand in a
        // single field yet. A whole-string search for "the first SEAMS+id" would
        // collapse this to one gear, silently dropping the other. Anchoring each
        // lookup at the occurrence it belongs to must resolve both.
        let r = resolve(
            "rating SEAMS M9 adopted; subscriptions SEAMS **SUB-P8** owed",
            &pricing(),
        );
        assert_eq!(
            r.paths,
            [
                "../../rating/docs/SEAMS.md",
                "../../subscriptions/docs/SEAMS.md",
            ]
        );
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn bare_seams_is_not_rescued_by_a_later_unrelated_id() {
        // A position-agnostic seam-id search would find "M12" — cited later, for a
        // different SEAMS occurrence — and wrongly use it to resolve the earlier,
        // genuinely bare SEAMS. The bare occurrence must land in unresolved on its
        // own merits, independent of what a later occurrence in the same string
        // happens to cite.
        let r = resolve(
            "SEAMS unspecified; rating SEAMS M12 confirms the same block",
            &pricing(),
        );
        assert_eq!(r.paths, ["../../rating/docs/SEAMS.md"]);
        assert_eq!(r.unresolved, ["SEAMS"]);
    }
}
