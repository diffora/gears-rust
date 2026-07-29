use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::Corpus;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub paths: Vec<String>,
    pub unresolved: Vec<String>,
    /// `SEAMS <id>` citations whose `id` is shaped like a seam id but which no loaded
    /// gear's `SEAMS.md` defines a row for — a dangling seam reference, which is a
    /// defect in the citing document, not a syntax miss (that's `unresolved`).
    pub seam_undefined: Vec<String>,
    /// `SEAMS <id>` citations whose `id` more than one loaded gear's `SEAMS.md` defines
    /// a row for, paired with the sorted, deduplicated gear names that claim it — two
    /// gears claiming one seam id is a genuine conflict, not a resolvable target.
    pub seam_conflicts: Vec<(String, Vec<String>)>,
}

/// Where a `SEAMS.md` seam id (`M12`, `RG3`, `SUB-P7`, …) is actually defined, gathered
/// once from every corpus the CLI loaded.
///
/// `resolve` looks a citation's id up here instead of inferring its owning gear from the
/// id's prefix. The id *shape* (uppercase, alphanumeric, optionally hyphenated) is a
/// documentation convention this tool may reasonably know; *which gears exist*, and
/// which of them owns a given id, is not — a new sibling gear needs no code change here,
/// only another `--gear` corpus at build time.
#[derive(Debug, Default)]
pub struct SeamIndex {
    /// Seam id -> the gear names (sorted, deduplicated) whose `SEAMS.md` defines a row
    /// for it. Almost always exactly one; more than one is a genuine cross-gear
    /// conflict, and zero (no entry at all) means no loaded corpus defines it.
    owners: BTreeMap<String, BTreeSet<String>>,
}

impl SeamIndex {
    /// Scans every corpus's top-level `SEAMS.md` for table rows shaped
    /// `| **<ID>** | … |` — the convention both the rating and subscriptions seam maps
    /// use. A corpus with no `SEAMS.md` (pricing has none; it only *cites* its
    /// neighbours' seam maps) simply contributes nothing — not an error.
    pub fn build(corpora: &[Corpus]) -> Self {
        let row = Regex::new(r"^\|\s*\*\*([A-Za-z0-9-]+)\*\*\s*\|").expect("valid seam-row regex");
        let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for corpus in corpora {
            let Some(text) = corpus.text("SEAMS.md") else {
                continue;
            };
            let Some(gear) = gear_name(corpus) else {
                continue;
            };
            for line in text.lines() {
                if let Some(c) = row.captures(line) {
                    owners
                        .entry(c[1].to_string())
                        .or_default()
                        .insert(gear.clone());
                }
            }
        }
        Self { owners }
    }

    /// Gear names that define a row for `id`, sorted; empty if none does.
    fn owners(&self, id: &str) -> Vec<&str> {
        self.owners
            .get(id)
            .map(|gears| gears.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }
}

/// The gear name a corpus belongs to: its root's `docs`-parent directory name
/// (`.../gears/bss/rating/docs` -> `rating`). `None` for a root shape this convention
/// doesn't apply to (e.g. a bare, single-component synthetic test root) — callers treat
/// that corpus as contributing nothing, never as an error.
fn gear_name(corpus: &Corpus) -> Option<String> {
    corpus
        .root()
        .parent()?
        .file_name()?
        .to_str()
        .map(str::to_string)
}

/// Maps the register's propagation shorthand onto corpus-relative paths.
///
/// Cross-gear targets (`SEAMS <id>`) are resolved against `seams` — the seam ids every
/// loaded gear corpus actually defines a row for — rather than inferred from the id's
/// prefix, and are returned as `../../<gear>/docs/SEAMS.md` (relative to `corpus`'s
/// root) so a finding names something a reader can open. A citation from within the
/// defining gear's own corpus resolves to the in-corpus `SEAMS.md` instead of escaping
/// and returning via `../../`. An id no loaded gear defines, or one two gears both
/// define, is reported on `Resolved` (`seam_undefined` / `seam_conflicts`) rather than
/// silently guessed.
pub fn resolve(raw: &str, corpus: &Corpus, seams: &SeamIndex) -> Resolved {
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
    //
    // The id shape itself (`[A-Z][A-Z0-9]*(-[A-Z0-9]+)*`) is gear-agnostic: it matches
    // every id family either live seam map actually uses (`K1`, `ASC`, `M12`, `RG3`,
    // `SUB-P7`, `UC6`, …), not just the two families (`M\d+`/`RG\d+` and `SUB-...`) a
    // prefix-inferring version would recognize. *Which gear* owns a captured id is
    // decided below by looking it up in `seams` — never by the shape.
    let seam_gear = Regex::new(r"^\s+(?:§\w+\s+)?\**([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*)\b")
        .expect("valid seam regex");

    let mut paths = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    let mut seam_undefined = BTreeSet::new();
    let mut seam_conflicts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let citing_gear = gear_name(corpus);

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
                Some(c) => {
                    let id = c[1].to_string();
                    let owners = seams.owners(&id);
                    match owners.len() {
                        0 => {
                            seam_undefined.insert(id);
                        }
                        1 => {
                            let gear = owners[0];
                            let path = if citing_gear.as_deref() == Some(gear) {
                                "SEAMS.md".to_string()
                            } else {
                                format!("../../{gear}/docs/SEAMS.md")
                            };
                            paths.insert(path);
                        }
                        _ => {
                            seam_conflicts
                                .entry(id)
                                .or_default()
                                .extend(owners.into_iter().map(|g| g.to_string()));
                        }
                    }
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
        seam_undefined: seam_undefined.into_iter().collect(),
        seam_conflicts: seam_conflicts
            .into_iter()
            .map(|(id, gears)| (id, gears.into_iter().collect()))
            .collect(),
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

    /// Loads a real gear's `docs/` corpus by name. Test-only: naming a gear here names a
    /// real corpus on disk for the test to load — it isn't resolution logic branching on
    /// a gear's identity, which is exactly what `SeamIndex` exists to avoid in
    /// production code.
    fn load_gear(name: &str) -> Corpus {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../gears/bss/{name}/docs"));
        Corpus::load(&root).expect("gear corpus loads")
    }

    /// The seam index the real CLI builds from all three live BSS gears.
    fn known_seams() -> SeamIndex {
        SeamIndex::build(&[
            load_gear("pricing"),
            load_gear("rating"),
            load_gear("subscriptions"),
        ])
    }

    #[test]
    fn resolves_slice_numbers_to_design_files() {
        let r = resolve("S7 (§1, W1, names)", &pricing(), &SeamIndex::default());
        assert_eq!(r.paths, ["design/07-pricewindow-linkage.md"]);
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn resolves_named_documents() {
        let r = resolve(
            "PRD §17.4; DESIGN §2; Foundation §4.3",
            &pricing(),
            &SeamIndex::default(),
        );
        assert_eq!(r.paths, ["DESIGN.md", "PRD.md", "design/01-foundation.md"]);
    }

    #[test]
    fn resolves_adrs_by_number() {
        let r = resolve(
            "ADR-0002 (new) + ADR-0001 amendment note",
            &pricing(),
            &SeamIndex::default(),
        );
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
        let r = resolve("SEAMS M12 asserts the block", &pricing(), &known_seams());
        assert_eq!(r.paths, ["../../rating/docs/SEAMS.md"]);

        let bare = resolve("SEAMS", &pricing(), &known_seams());
        assert!(bare.paths.is_empty());
        assert_eq!(bare.unresolved, ["SEAMS"]);
    }

    #[test]
    fn deduplicates_and_sorts() {
        let r = resolve("PRD §1; PRD §2; S3", &pricing(), &SeamIndex::default());
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
        let r = resolve(
            "subscriptions SEAMS **SUB-P7**.",
            &pricing(),
            &known_seams(),
        );
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
        //
        // M9 and SUB-P7 are real, singly-defined ids (rating and subscriptions
        // respectively): under a prefix-inferring resolver any `M\d+`/`SUB-...`-shaped
        // text would do, but the defined-by-a-loaded-corpus lookup needs ids that
        // actually are defined somewhere.
        let r = resolve(
            "rating SEAMS M9 adopted; subscriptions SEAMS **SUB-P7** owed",
            &pricing(),
            &known_seams(),
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
            &known_seams(),
        );
        assert_eq!(r.paths, ["../../rating/docs/SEAMS.md"]);
        assert_eq!(r.unresolved, ["SEAMS"]);
    }

    #[test]
    fn seam_citation_within_the_defining_gear_resolves_to_a_same_corpus_path() {
        // subscriptions' own DECISIONS.md (SUB-D-01) cites its own SUB-R1 row this
        // exact way. The resolved path must stay in-corpus (`SEAMS.md`) rather than
        // escaping via `../../subscriptions/docs/` back to the very corpus it started
        // in.
        let r = resolve(
            "SEAMS SUB-R1 note",
            &load_gear("subscriptions"),
            &known_seams(),
        );
        assert_eq!(r.paths, ["SEAMS.md"]);
        assert!(r.unresolved.is_empty());
        assert!(r.seam_undefined.is_empty());
    }

    #[test]
    fn seam_id_defined_by_no_loaded_gear_is_reported_as_undefined() {
        // "Z9" is shaped like a seam id (so this isn't a syntactic `unresolved` miss)
        // but is not a row any real SEAMS.md defines — a dangling citation, which is a
        // defect in the citing document, not a resolver gap.
        let r = resolve("SEAMS Z9 pending", &pricing(), &known_seams());
        assert!(r.paths.is_empty());
        assert!(r.unresolved.is_empty());
        assert_eq!(r.seam_undefined, ["Z9"]);
    }

    #[test]
    fn seam_id_defined_by_two_gears_is_reported_as_a_conflict() {
        // No two real gears currently claim the same seam id (verified against the
        // live corpus), so this constructs the conflict directly: two synthetic
        // corpora, each defining a `Z1` row.
        let alpha = Corpus::from_parts(
            "gears/bss/alpha/docs",
            [(
                "SEAMS.md",
                "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n\
                 | **Z1** | HIGH | Joint | Alpha's definition. |\n",
            )],
        );
        let beta = Corpus::from_parts(
            "gears/bss/beta/docs",
            [(
                "SEAMS.md",
                "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n\
                 | **Z1** | HIGH | Joint | Beta's conflicting definition. |\n",
            )],
        );
        let seams = SeamIndex::build(&[alpha, beta]);

        let r = resolve("rating SEAMS Z1 update", &pricing(), &seams);
        assert!(r.paths.is_empty(), "unexpected: {r:#?}");
        assert_eq!(
            r.seam_conflicts,
            [(
                "Z1".to_string(),
                vec!["alpha".to_string(), "beta".to_string()]
            )]
        );
    }
}
