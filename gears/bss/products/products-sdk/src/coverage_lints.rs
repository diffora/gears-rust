//! `CoverageChecks` — lints 1 to 8 of `design/12` §3.2, executable, on the
//! grammars **P-D-130** stated once (P-D-43, P-D-45 before it). Lint 9 is
//! `pin_lint.rs`; the two P-D-130 lints (reciprocity, PII-hook) stay declared
//! and unenforced.
//!
//! Like lint 9 these ride `cargo test` of this crate rather than a CI job:
//! the owner refused a job (P-D-132, `dod-lint-gate` stays open by that
//! decision), so a lint that fails **in the change that broke it** is the
//! enforcement available. They live under `src/` for lint 9's reason — the
//! traceability scanner knows no `-sdk/tests` root.
//!
//! # The corpus, and the grammars
//!
//! The corpus is the design set (`docs/design/NN-*.md`) plus `PRD.md`;
//! `DECOMPOSITION.md`, `DESIGN.md` and the index are out (P-D-130), as are the
//! FEATURE documents, which restate the slices and would double every count.
//!
//! - **Lint 1** (requirement coverage): the universe is `PRD.md`'s `p1`/`p2`
//!   requirement-bearing ids — `fr-*`, `nfr-*`, `interface-*`, `contract-*`
//!   and the `usecase-*` ids, harvested from the PRD alone (the design set
//!   declares fourteen `contract-*` ids of its own). A claim is a backticked
//!   id in a slice's `**Traces to**` paragraph with an optional parenthesised
//!   qualifier; qualifiers compare as case-folded strings; an unqualified claim
//!   conflicts with every other claim of the id; two identical qualifiers
//!   conflict. Multiply claimed means n ≥ 2 distinct qualifiers. The
//!   AC-existence half: every `AC #N` cited in a slice resolves to a §12
//!   bold-numbered item unless the sentence names another gear.
//! - **Lint 2** (the AC #38 map): `design/12` §4.1's table carries fifteen rows
//!   (a transcribed constant, P-D-130); every code cell is a code this SDK's
//!   [`crate::errors::ErrorCode`] vocabulary carries — the doc-to-code pin —
//!   the excluded rows are exactly 8, 14 and 15, and each mapped code's
//!   declaring slice names it in its own error-taxonomy section.
//! - **Lint 3** (door×grant): the population is every `` `VERB /bss-products/v1/…` ``
//!   span in the corpus, normalised — `\|` → `|`, `{products|skus}` expanded,
//!   path parameters compared by position (`{id}` = `{skuId}`), a query string
//!   or an ellipsis dropped (P-D-151 extends P-D-130's three normalisations by
//!   these two, measured necessary) — and every member appears in `design/05`
//!   §3.2's `Door(s)` column.
//! - **Lint 4** (event bookkeeping): every row of `design/12` §4.2's
//!   `EventRegister` names an instruction the design set declares, and the
//!   declaring slice's text names the event; every token this SDK's
//!   [`crate::events::SCHEMA_REFS`] versions has a register row.
//! - **Lint 5** (register hygiene): every `- **Propagated**` field of a `P-D`
//!   entry names documents in the corpus grammar (`PRD`, `design/NN`, `NN`, a
//!   slice or feature file, `DESIGN.md`, `DECOMPOSITION`) and every named
//!   document **cites the id** — filings, not citers (P-D-128/P-D-130); an
//!   `S<NN>` abbreviation is illegal (`design/12` §3.2).
//! - **Lint 6** (id uniqueness): every `inst-*` id is declared exactly once — a
//!   declaration is the id trailing its own numbered instruction row; a
//!   parenthesised `(cont. inst-…)` is a continuation, never a declaration.
//! - **Lint 7** (identity materialization): exactly one table declaration in
//!   the slices' §4 sections holds a real-identity column (`principal_ref`,
//!   `principal_id`, `subject_id`, `operator_identity`) and it is
//!   `products_identity_ref`; a pseudonymous `actor_ref` is not an identity.
//! - **Lint 8** (no monetization marker): no backticked identifier in a §4
//!   section names `flat`, `per-seat`/`per_seat`, `tiered`, `volume`, `hybrid`
//!   or `commitment`; `usage` is admitted (AC #37).
//!
//! Each lint has a failing case beside its passing case, on a perturbed copy
//! of its input — a lint asserted only by passing proves nothing.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-lint-prd-universe:p1
//! @cpt-dod:cpt-cf-bss-products-dod-lint-declarations:p1
//! @cpt-dod:cpt-cf-bss-products-dod-lint-surfaces:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// The corpus is written in typographic Unicode (`§`, `×`, `—`, `…`); a lint
// that reads it must spell what it reads.
#![allow(clippy::non_ascii_literal)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::errors::ErrorCode;
use crate::events::SCHEMA_REFS;

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

fn docs_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("the corpus must be readable: {}: {e}", path.display()))
}

/// `(two-digit slice number, file name, text)` for every design slice.
fn slices() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(docs_root().join("design")).expect("design/ lists") {
        let path = entry.expect("entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let is_md = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if !is_md || !name.as_bytes()[0].is_ascii_digit() {
            continue;
        }
        out.push((name[..2].to_owned(), name.clone(), read(&path)));
    }
    out.sort();
    assert!(
        out.len() >= 12,
        "the design set has twelve slices; found {}",
        out.len()
    );
    out
}

fn prd() -> String {
    read(&docs_root().join("PRD.md"))
}

fn section<'a>(text: &'a str, heading_prefix: &str) -> Option<&'a str> {
    let start = text.find(heading_prefix)?;
    let rest = &text[start..];
    let body_start = rest.find('\n').map_or(rest.len(), |i| i + 1);
    let level = rest
        .trim_start_matches('\n')
        .chars()
        .take_while(|c| *c == '#')
        .count();
    assert!(level > 0, "a heading prefix: {heading_prefix:?}");
    let mut end = rest.len();
    for (i, line) in rest[body_start..]
        .match_indices('\n')
        .map(|(i, _)| (i, &rest[body_start + i + 1..]))
    {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes <= level && line.chars().nth(hashes) == Some(' ') {
            end = body_start + i + 1;
            break;
        }
    }
    Some(&rest[..end])
}

// ---------------------------------------------------------------------------
// Lint 1 — requirement coverage and AC existence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Claim {
    slice: String,
    qualifier: Option<String>,
}

fn prd_requirement_ids(prd: &str) -> BTreeMap<String, String> {
    let re = Regex::new(
        r"(?m)^- \[.\] `(p[123])` - \*\*ID\*\*: `(cpt-cf-bss-products-(?:fr|nfr|interface|contract|usecase)-[a-z0-9-]+)`",
    )
    .unwrap();
    re.captures_iter(prd)
        .map(|c| (c[2].to_owned(), c[1].to_owned()))
        .collect()
}

fn claims_of(slice: &str, text: &str) -> Vec<(String, Claim)> {
    let para = Regex::new(r"(?ms)^\*\*Traces to\*\*:(.*?)(?:\n\n|\z)")
        .unwrap()
        .captures(text)
        .map(|c| c[1].to_owned())
        .unwrap_or_default();
    let re = Regex::new(
        r"`(cpt-cf-bss-products-(?:fr|nfr|interface|contract|usecase)-[a-z0-9-]+)`(?:\s*\(((?:[^()]|\([^()]*\))*)\))?",
    )
    .unwrap();
    re.captures_iter(&para)
        .map(|c| {
            (
                c[1].to_owned(),
                Claim {
                    slice: slice.to_owned(),
                    qualifier: c.get(2).map(|q| q.as_str().trim().to_lowercase()),
                },
            )
        })
        .collect()
}

/// Lint 1's verdict over a universe and a claim list: the conflicts.
fn requirement_conflicts(
    universe: &BTreeMap<String, String>,
    claims: &[(String, Claim)],
) -> Vec<String> {
    let mut by_id: BTreeMap<&str, Vec<&Claim>> = BTreeMap::new();
    for (id, claim) in claims {
        by_id.entry(id.as_str()).or_default().push(claim);
    }
    let mut findings = Vec::new();
    for (id, priority) in universe {
        if priority == "p3" {
            continue;
        }
        let Some(list) = by_id.get(id.as_str()) else {
            findings.push(format!("{id} ({priority}) is claimed by no slice"));
            continue;
        };
        if list.len() < 2 {
            continue;
        }
        if list.iter().any(|c| c.qualifier.is_none()) {
            findings.push(format!(
                "{id} is claimed by {} slices and one claim is unqualified",
                list.len()
            ));
        }
        // Qualifiers compare as case-folded strings (P-D-130).
        let quals: BTreeSet<String> = list
            .iter()
            .filter_map(|c| c.qualifier.as_deref().map(str::to_lowercase))
            .collect();
        let qualified = list.iter().filter(|c| c.qualifier.is_some()).count();
        if quals.len() < qualified {
            findings.push(format!("{id} carries two identical qualifiers"));
        }
    }
    for id in by_id.keys() {
        if !universe.contains_key(*id) {
            findings.push(format!("{id} is claimed but is not a PRD requirement id"));
        }
    }
    findings
}

fn prd_ac_count(prd: &str) -> usize {
    let sec = section(prd, "\n## 12. ").expect("PRD §12");
    Regex::new(r"(?m)^\*\*(\d+)\. ")
        .unwrap()
        .captures_iter(sec)
        .map(|c| c[1].parse::<usize>().unwrap())
        .max()
        .expect("§12 numbers its criteria")
}

/// `AC #N` citations whose sentence names no other gear.
fn unqualified_ac_citations(text: &str) -> Vec<usize> {
    let re = Regex::new(r"AC\s*#(\d+)").unwrap();
    let gear =
        Regex::new(r"(?i)\b(pricing|subscriptions|rating|ledger|contracts|billing)\b").unwrap();
    let mut out = Vec::new();
    for m in re.captures_iter(text) {
        let at = m.get(0).unwrap().start();
        // The sentence context: back to the previous sentence end or
        // paragraph break, at most 240 chars.
        let from = at.saturating_sub(240);
        let window = &text[from..at];
        let sentence_start = window.rfind(['.', ';', '\n']).map_or(0, |i| i + 1);
        if gear.is_match(&window[sentence_start..]) {
            continue;
        }
        out.push(m[1].parse::<usize>().unwrap());
    }
    out
}

#[test]
fn lint_1_every_p1_p2_requirement_has_one_owner_per_clause() {
    let prd = prd();
    let universe = prd_requirement_ids(&prd);
    assert_eq!(universe.len(), 71, "the PRD's requirement universe");
    let mut claims = Vec::new();
    for (slice, _, text) in slices() {
        claims.extend(claims_of(&slice, &text));
    }
    let findings = requirement_conflicts(&universe, &claims);
    assert!(findings.is_empty(), "lint 1: {findings:#?}");

    // The multiply-claimed set is admitted, the triple included.
    let mut by_id: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (id, claim) in &claims {
        by_id
            .entry(id.as_str())
            .or_default()
            .insert(claim.slice.as_str());
    }
    let multi = by_id.values().filter(|s| s.len() > 1).count();
    assert_eq!(multi, 14, "fourteen requirements are multiply claimed");
    let triple = by_id["cpt-cf-bss-products-nfr-scale-extensibility"].clone();
    assert_eq!(triple, BTreeSet::from(["01", "02", "06"]), "the one triple");

    // The AC-existence half.
    let count = prd_ac_count(&prd);
    for (slice, _, text) in slices() {
        for n in unqualified_ac_citations(&text) {
            assert!(
                n >= 1 && n <= count,
                "slice {slice} cites AC #{n}; §12 has {count}"
            );
        }
    }
}

#[test]
fn lint_1_fails_on_an_unqualified_claim_beside_a_qualified_one_and_on_twins() {
    let universe = BTreeMap::from([("cpt-cf-bss-products-fr-x".to_owned(), "p1".to_owned())]);
    let q = |s: &str, q: Option<&str>| {
        (
            "cpt-cf-bss-products-fr-x".to_owned(),
            Claim {
                slice: s.to_owned(),
                qualifier: q.map(str::to_owned),
            },
        )
    };
    assert!(requirement_conflicts(&universe, &[q("01", Some("a")), q("02", Some("b"))]).is_empty());
    let unq = requirement_conflicts(&universe, &[q("01", Some("a")), q("02", None)]);
    assert!(unq.iter().any(|f| f.contains("unqualified")), "{unq:?}");
    let twins = requirement_conflicts(&universe, &[q("01", Some("a")), q("02", Some("A"))]);
    assert!(twins.iter().any(|f| f.contains("identical")), "{twins:?}");
    let none = requirement_conflicts(&universe, &[]);
    assert!(none.iter().any(|f| f.contains("no slice")), "{none:?}");
    // A gear-qualified citation is outside the AC-existence check.
    assert_eq!(
        unqualified_ac_citations("pricing AC #82's own When. AC #3 here; rating AC #9."),
        vec![3]
    );
}

// ---------------------------------------------------------------------------
// Lint 2 — the AC #38 map
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct MapRow {
    number: usize,
    code: Option<String>,
    slice: Option<String>,
}

fn ac38_map(design_12: &str) -> Vec<MapRow> {
    let sec = section(design_12, "\n### 4.1 ").expect("design/12 §4.1");
    let re = Regex::new(r"(?m)^\| (\d+) \| [^|]+ \| ([^|]+) \| ([^|]+) \|").unwrap();
    re.captures_iter(sec)
        .map(|c| MapRow {
            number: c[1].parse().unwrap(),
            code: Regex::new(r"`([A-Z_]+)`")
                .unwrap()
                .captures(&c[2])
                .map(|m| m[1].to_owned()),
            slice: Regex::new(r"\b(\d\d)\b")
                .unwrap()
                .captures(&c[3])
                .map(|m| m[1].to_owned()),
        })
        .collect()
}

fn ac38_findings(rows: &[MapRow], taxonomy_of: &BTreeMap<String, String>) -> Vec<String> {
    let mut f = Vec::new();
    if rows.len() != 15 {
        f.push(format!("the map carries {} rows, not fifteen", rows.len()));
    }
    let excluded: BTreeSet<usize> = rows
        .iter()
        .filter(|r| r.code.is_none())
        .map(|r| r.number)
        .collect();
    if excluded != BTreeSet::from([8, 14, 15]) {
        f.push(format!(
            "the excluded rows are {excluded:?}, not exactly 8, 14 and 15"
        ));
    }
    let mut declaring: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in rows {
        let Some(code) = row.code.as_deref() else {
            continue;
        };
        if ErrorCode::parse(code).is_none() {
            f.push(format!(
                "row {}: `{code}` is not a registered code",
                row.number
            ));
        }
        let Some(slice) = row.slice.as_deref() else {
            f.push(format!(
                "row {}: `{code}` names no declaring slice",
                row.number
            ));
            continue;
        };
        declaring.entry(code).or_default().insert(slice);
        match taxonomy_of.get(slice) {
            Some(tax) if tax.contains(&format!("`{code}`")) => {}
            _ => f.push(format!(
                "row {}: slice {slice}'s error taxonomy does not name `{code}`",
                row.number
            )),
        }
    }
    for (code, slices) in declaring {
        if slices.len() != 1 {
            f.push(format!(
                "`{code}` is declared by {slices:?}: one declaring slice, not several"
            ));
        }
    }
    f
}

fn taxonomy_sections() -> BTreeMap<String, String> {
    let heading = Regex::new(r"(?m)^### \d\.\d Error taxonomy").unwrap();
    slices()
        .into_iter()
        .filter_map(|(n, _, text)| {
            // The heading itself, not the table of contents' mention of it.
            let start = heading.find(&text)?.start();
            let sec = section(&text[start.saturating_sub(1)..], "\n### ")?;
            Some((n, sec.to_owned()))
        })
        .collect()
}

#[test]
fn lint_2_the_ac38_map_is_fifteen_rows_twelve_registered_codes_three_named_exclusions() {
    let d12 = slices().into_iter().find(|(n, _, _)| n == "12").unwrap().2;
    let rows = ac38_map(&d12);
    let findings = ac38_findings(&rows, &taxonomy_sections());
    assert!(findings.is_empty(), "lint 2: {findings:#?}");
    assert_eq!(rows.iter().filter(|r| r.code.is_some()).count(), 12);
}

#[test]
fn lint_2_fails_on_a_fourth_exclusion_an_unknown_code_and_a_second_declaring_slice() {
    let d12 = slices().into_iter().find(|(n, _, _)| n == "12").unwrap().2;
    let tax = taxonomy_sections();
    let mut rows = ac38_map(&d12);
    rows[0].code = None; // row 1 excluded without explanation
    let f = ac38_findings(&rows, &tax);
    assert!(f.iter().any(|x| x.contains("excluded rows")), "{f:?}");
    let mut rows = ac38_map(&d12);
    rows[0].code = Some("NOT_A_CODE".to_owned());
    let f = ac38_findings(&rows, &tax);
    assert!(
        f.iter().any(|x| x.contains("not a registered code")),
        "{f:?}"
    );
    let mut rows = ac38_map(&d12);
    // `UNRECOGNIZED_UNIT` is on rows 4 and 11, both 03: move one to 01.
    let row11 = rows.iter_mut().find(|r| r.number == 11).unwrap();
    row11.slice = Some("01".to_owned());
    let f = ac38_findings(&rows, &tax);
    assert!(f.iter().any(|x| x.contains("one declaring slice")), "{f:?}");
}

// ---------------------------------------------------------------------------
// Lint 3 — door×grant pairing
// ---------------------------------------------------------------------------

fn normalise_route(verb: &str, path: &str) -> Vec<String> {
    let path = path.replace("\\|", "|");
    let path = path
        .split('?')
        .next()
        .unwrap()
        .trim_end_matches('…')
        .to_owned();
    let variants = if path.contains("{products|skus}") {
        vec![
            path.replace("{products|skus}", "products"),
            path.replace("{products|skus}", "skus"),
        ]
    } else {
        vec![path]
    };
    let param = Regex::new(r"\{[^}]*\}").unwrap();
    variants
        .into_iter()
        .map(|p| format!("{verb} {}", param.replace_all(&p, "{}")))
        .collect()
}

fn route_spans(text: &str) -> BTreeSet<String> {
    let re = Regex::new(r"`(GET|POST|PATCH|PUT|DELETE) (/bss-products/v1/[^`]*)`").unwrap();
    re.captures_iter(text)
        .flat_map(|c| normalise_route(&c[1], &c[2]))
        .collect()
}

fn door_grant_findings(declared: &BTreeSet<String>, doors: &BTreeSet<String>) -> Vec<String> {
    declared
        .difference(doors)
        .map(|r| format!("{r} is declared and paired with no grant"))
        .collect()
}

#[test]
fn lint_3_every_declared_route_appears_in_the_rbac_catalogs_door_column() {
    let mut declared = BTreeSet::new();
    let mut doors = BTreeSet::new();
    for (n, _, text) in slices() {
        declared.extend(route_spans(&text));
        if n == "05" {
            let sec = section(&text, "\n### 3.2 ").expect("design/05 §3.2");
            doors = route_spans(sec);
        }
    }
    declared.extend(route_spans(&prd()));
    assert!(
        declared.len() >= 50,
        "the declared population is {}",
        declared.len()
    );
    let findings = door_grant_findings(&declared, &doors);
    assert!(findings.is_empty(), "lint 3: {findings:#?}");
}

#[test]
fn lint_3_pairs_the_pipe_bearing_routes_across_the_escaped_and_unescaped_forms() {
    let escaped = route_spans("`GET /bss-products/v1/{products\\|skus}/{id}`");
    let plain = route_spans("`GET /bss-products/v1/{products|skus}/{skuId}`");
    assert_eq!(escaped, plain);
    assert_eq!(escaped.len(), 2, "one span, two routes");
    let doors = route_spans("`GET /bss-products/v1/products/{id}`");
    let f = door_grant_findings(&plain, &doors);
    assert_eq!(f.len(), 1, "the SKU half is unpaired: {f:?}");
    assert!(f[0].starts_with("GET /bss-products/v1/skus/{}"));
    // A query string and an ellipsis are not part of the route.
    assert_eq!(
        route_spans("`GET /bss-products/v1/approvals?state=pending`"),
        route_spans("`GET /bss-products/v1/approvals`")
    );
    assert_eq!(
        route_spans("`GET /bss-products/v1/browse…`"),
        route_spans("`GET /bss-products/v1/browse`")
    );
}

// ---------------------------------------------------------------------------
// Lint 4 — event bookkeeping over the EventRegister
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisterRow {
    event: String,
    instruction: Option<String>,
    slice: String,
}

fn event_register(design_12: &str) -> Vec<RegisterRow> {
    let sec = section(design_12, "\n### 4.2 ").expect("design/12 §4.2");
    let re = Regex::new(r"(?m)^\| `?([A-Za-z]+|—)`? \| ([^|]+) \| (\d\d) \|").unwrap();
    re.captures_iter(sec)
        .map(|c| RegisterRow {
            event: c[1].to_owned(),
            instruction: Regex::new(r"`(inst-[a-z0-9-]+)`")
                .unwrap()
                .captures(&c[2])
                .map(|m| m[1].to_owned()),
            slice: c[3].to_owned(),
        })
        .collect()
}

fn declared_instructions(text: &str) -> BTreeMap<String, String> {
    // A declaration is an instruction item — numbered or nested, at any
    // indentation — whose trailing token is the id (lint 6's grammar). Its
    // block runs from the item's head line through every line indented
    // deeper than the head, so a sub-bullet roster under the row belongs to
    // the row.
    let mut out = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let head = Regex::new(r"^(\s*)(?:\d+\.|-) \[.\] - `p\d` - ").unwrap();
    let id = Regex::new(r"- `(inst-[a-z0-9-]+)`\s*$").unwrap();
    for (i, line) in lines.iter().enumerate() {
        let Some(m) = id.captures(line) else { continue };
        // Walk back to the item head this id closes.
        let mut start = i;
        while start > 0 && !head.is_match(lines[start]) {
            start -= 1;
        }
        let indent = head.captures(lines[start]).map_or(0, |c| c[1].len());
        // Run forward over the continuation: deeper-indented lines.
        let mut end = i + 1;
        while end < lines.len() {
            let l = lines[end];
            let l_indent = l.len() - l.trim_start().len();
            if l.trim().is_empty() || (l_indent > indent && !l.starts_with('#')) {
                end += 1;
            } else {
                break;
            }
        }
        out.insert(m[1].to_owned(), lines[start..end].join("\n"));
    }
    out
}

fn register_findings(
    rows: &[RegisterRow],
    instructions: &BTreeMap<String, (String, String)>,
) -> Vec<String> {
    let mut f = Vec::new();
    if rows.is_empty() {
        f.push(
            "the EventRegister is empty: every row of an empty table trivially names nothing"
                .to_owned(),
        );
    }
    let mut events = BTreeSet::new();
    for row in rows {
        let Some(inst) = row.instruction.as_deref() else {
            f.push(format!("{}: no emitting instruction", row.event));
            continue;
        };
        let Some((slice, block)) = instructions.get(inst) else {
            f.push(format!("{}: `{inst}` is declared by no slice", row.event));
            continue;
        };
        if *slice != row.slice {
            f.push(format!(
                "{}: `{inst}` is slice {slice}'s, the row says {}",
                row.event, row.slice
            ));
        }
        if row.event != "—" {
            events.insert(row.event.as_str());
            if !block.contains(&row.event) {
                f.push(format!(
                    "{}: `{inst}`'s own row does not name the event",
                    row.event
                ));
            }
        }
    }
    for (token, _) in SCHEMA_REFS {
        if !events.contains(token) {
            f.push(format!(
                "{token} is versioned by the SDK roster and has no register row"
            ));
        }
    }
    f
}

fn all_instructions() -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for (n, _, text) in slices() {
        for (id, block) in declared_instructions(&text) {
            out.insert(id, (n.clone(), block));
        }
    }
    out
}

#[test]
fn lint_4_every_register_row_names_its_emitting_instruction_and_every_event_is_registered() {
    let d12 = slices().into_iter().find(|(n, _, _)| n == "12").unwrap().2;
    let rows = event_register(&d12);
    assert!(rows.len() >= 40, "the register carries {} rows", rows.len());
    let findings = register_findings(&rows, &all_instructions());
    assert!(findings.is_empty(), "lint 4: {findings:#?}");
}

#[test]
fn lint_4_fails_on_an_empty_register_an_unknown_instruction_and_an_unregistered_event() {
    let inst = all_instructions();
    let f = register_findings(&[], &inst);
    assert!(f.iter().any(|x| x.contains("empty")), "{f:?}");
    let d12 = slices().into_iter().find(|(n, _, _)| n == "12").unwrap().2;
    let mut rows = event_register(&d12);
    rows[0].instruction = Some("inst-zz-nowhere".to_owned());
    let f = register_findings(&rows, &inst);
    assert!(
        f.iter().any(|x| x.contains("declared by no slice")),
        "{f:?}"
    );
    let mut rows = event_register(&d12);
    rows.retain(|r| r.event != "SkuRetired");
    let f = register_findings(&rows, &inst);
    assert!(
        f.iter()
            .any(|x| x.contains("SkuRetired") && x.contains("no register row")),
        "{f:?}"
    );
}

// ---------------------------------------------------------------------------
// Lint 5 — register hygiene
// ---------------------------------------------------------------------------

fn corpus_docs() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_, name, text) in slices() {
        out.insert(name, text);
    }
    out.insert("PRD.md".to_owned(), prd());
    for name in ["DESIGN.md", "DECOMPOSITION.md"] {
        out.insert(name.to_owned(), read(&docs_root().join(name)));
    }
    for entry in std::fs::read_dir(docs_root().join("features")).expect("features/ lists") {
        let path = entry.expect("entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.insert(format!("features/{name}"), read(&path));
        }
    }
    out
}

/// A propagated document name, resolved to a corpus key; `Err` for an illegal
/// spelling, `Ok(None)` for a token that is not a document name at all.
fn resolve_document(
    token: &str,
    docs: &BTreeMap<String, String>,
) -> Result<Option<String>, String> {
    let t = token.trim().trim_matches('`').trim();
    if Regex::new(r"^S\d\d\b").unwrap().is_match(t) {
        return Err(format!(
            "`{t}`: an S<NN> abbreviation is not a document name"
        ));
    }
    if t == "PRD" || t == "PRD.md" || t.starts_with("PRD §") || t.starts_with("PRD.md §") {
        return Ok(Some("PRD.md".to_owned()));
    }
    for name in ["DESIGN.md", "DECOMPOSITION.md"] {
        if t == name || t == name.trim_end_matches(".md") || t.starts_with(&format!("{name} §")) {
            return Ok(Some(name.to_owned()));
        }
    }
    if let Some(c) = Regex::new(r"^design/(\d\d)(?:-[a-z-]+)?(?:\.md)?(?:\s*§.*)?$")
        .unwrap()
        .captures(t)
    {
        let slice = docs
            .keys()
            .find(|k| k.starts_with(&format!("{}-", &c[1])))
            .cloned();
        return Ok(slice);
    }
    if let Some(c) = Regex::new(r"^(\d\d)(?:\s*§.*)?$").unwrap().captures(t) {
        // A bare slice number names the slice pair: the design document or
        // its feature document (P-D-151 reading P-D-130's "as one set").
        let pair: Vec<String> = docs
            .keys()
            .filter(|k| {
                k.starts_with(&format!("{}-", &c[1])) || {
                    docs.keys().any(|d| {
                        d.starts_with(&format!("{}-", &c[1]))
                            && k.as_str() == format!("features/{}", &d[3..])
                    })
                }
            })
            .cloned()
            .collect();
        return Ok(pair.first().map(|_| format!("pair:{}", &c[1])));
    }
    if let Some(c) = Regex::new(r"^features/([a-z-]+\.md)").unwrap().captures(t) {
        let key = format!("features/{}", &c[1]);
        return Ok(docs.contains_key(&key).then_some(key));
    }
    Ok(None)
}

/// Every `- **Propagated**` field of one entry: the bullet's text through its
/// continuation lines, up to the next bullet or blank line.
fn propagated_fields(entry: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in entry.lines() {
        if let Some(rest) = line.strip_prefix("- **Propagated**") {
            if let Some(done) = current.take() {
                out.push(done);
            }
            current = Some(rest.trim_start_matches(':').to_owned());
        } else if let Some(text) = current.as_mut() {
            if line.starts_with("  ") {
                text.push('\n');
                text.push_str(line);
            } else {
                out.push(current.take().unwrap());
            }
        }
    }
    if let Some(done) = current {
        out.push(done);
    }
    out
}

fn propagation_findings(decisions: &str, docs: &BTreeMap<String, String>) -> Vec<String> {
    let mut f = Vec::new();
    let entry = Regex::new(r"(?m)^#### (P-D-\d+)").unwrap();
    let starts: Vec<(usize, String)> = entry
        .captures_iter(decisions)
        .map(|c| (c.get(0).unwrap().start(), c[1].to_owned()))
        .collect();
    let token = Regex::new(r"`([^`]+)`|\b(PRD(?:\.md)?(?: §[0-9.]+)?|DESIGN\.md|DECOMPOSITION(?:\.md)?|design/\d\d[a-z-]*(?:\.md)?|features/[a-z-]+\.md)").unwrap();
    for (i, (start, id)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(decisions.len(), |(s, _)| *s);
        let body = &decisions[*start..end];
        let mut fields = 0;
        for field_text in propagated_fields(body) {
            fields += 1;
            for t in token.captures_iter(&field_text) {
                let tok = t.get(1).or_else(|| t.get(2)).unwrap().as_str();
                match resolve_document(tok, docs) {
                    Err(e) => f.push(format!("{id}: {e}")),
                    Ok(None) => {}
                    Ok(Some(doc)) if doc.starts_with("pair:") => {
                        let n = &doc[5..];
                        let cited = docs.iter().any(|(k, text)| {
                            (k.starts_with(&format!("{n}-"))
                                || docs.keys().any(|d| {
                                    d.starts_with(&format!("{n}-"))
                                        && *k == format!("features/{}", &d[3..])
                                }))
                                && text.contains(id.as_str())
                        });
                        if !cited {
                            f.push(format!(
                                "{id}: names slice {n}, neither of whose documents cites it"
                            ));
                        }
                    }
                    Ok(Some(doc)) => {
                        if !docs[&doc].contains(id.as_str()) {
                            f.push(format!("{id}: names {doc}, which does not cite it"));
                        }
                    }
                }
            }
        }
        if fields == 0 {
            f.push(format!("{id}: no `- **Propagated**` field"));
        }
    }
    f
}

#[test]
fn lint_5_every_propagated_document_cites_the_decision_and_every_entry_has_the_field() {
    let decisions = read(&docs_root().join("DECISIONS.md"));
    let findings = propagation_findings(&decisions, &corpus_docs());
    assert!(findings.is_empty(), "lint 5: {findings:#?}");
}

#[test]
fn lint_5_fails_on_an_uncited_filing_a_missing_field_and_an_abbreviation() {
    let docs = BTreeMap::from([
        ("PRD.md".to_owned(), "cites P-D-1".to_owned()),
        ("05-governance.md".to_owned(), "cites nothing".to_owned()),
    ]);
    let ok = "#### P-D-1 — x\n- **Propagated**: PRD §2\n";
    assert!(propagation_findings(ok, &docs).is_empty());
    let uncited = "#### P-D-1 — x\n- **Propagated**: `design/05` §3\n";
    let f = propagation_findings(uncited, &docs);
    assert!(f.iter().any(|x| x.contains("does not cite")), "{f:?}");
    let missing = "#### P-D-2 — x\n- **Date**: today\n";
    let f = propagation_findings(missing, &docs);
    assert!(
        f.iter().any(|x| x.contains("no `- **Propagated**` field")),
        "{f:?}"
    );
    let abbrev = "#### P-D-1 — x\n- **Propagated**: `S05` §3\n";
    let f = propagation_findings(abbrev, &docs);
    assert!(f.iter().any(|x| x.contains("abbreviation")), "{f:?}");
}

// ---------------------------------------------------------------------------
// Lint 6 — inst-* uniqueness
// ---------------------------------------------------------------------------

fn declaration_counts(text: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    let id = Regex::new(r"- `(inst-[a-z0-9-]+)`\s*$").unwrap();
    for line in text.lines() {
        if let Some(m) = id.captures(line) {
            *out.entry(m[1].to_owned()).or_insert(0) += 1;
        }
    }
    out
}

fn uniqueness_findings(counts: &BTreeMap<String, usize>) -> Vec<String> {
    counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(id, n)| format!("`{id}` is declared {n} times"))
        .collect()
}

/// Every `inst-*` id the design set mentions; an id mentioned and never
/// declared is a row whose id stopped trailing it (text appended after the
/// id), or a dangling reference — both are lint 6's business, since
/// "declared exactly once" is false at zero as at two.
fn mentioned_instructions(text: &str) -> BTreeSet<String> {
    Regex::new(r"`(inst-[a-z0-9-]+)`")
        .unwrap()
        .captures_iter(text)
        .map(|c| c[1].to_owned())
        .collect()
}

#[test]
fn lint_6_every_instruction_id_is_declared_once() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, text) in slices() {
        for (id, n) in declaration_counts(&text) {
            *counts.entry(id).or_insert(0) += n;
        }
    }
    assert!(
        counts.len() >= 260,
        "{} declared instruction ids",
        counts.len()
    );
    let mut findings = uniqueness_findings(&counts);
    let mut mentioned = BTreeSet::new();
    for (_, _, text) in slices() {
        mentioned.extend(mentioned_instructions(&text));
    }
    for id in mentioned.difference(&counts.keys().cloned().collect()) {
        findings.push(format!("`{id}` is mentioned and declared zero times"));
    }
    assert!(findings.is_empty(), "lint 6: {findings:#?}");
}

#[test]
fn lint_6_admits_a_continuation_row_and_fails_a_second_bare_declaration() {
    let text = "1. [ ] - `p1` - first - `inst-x-one`\n2. [ ] - `p1` - more (cont. inst-x-one) - `inst-x-two`\n";
    assert!(uniqueness_findings(&declaration_counts(text)).is_empty());
    let twice = "1. [ ] - `p1` - first - `inst-x-one`\n2. [ ] - `p1` - again - `inst-x-one`\n";
    let f = uniqueness_findings(&declaration_counts(twice));
    assert_eq!(f, vec!["`inst-x-one` is declared 2 times"]);
    // An id followed by more text on its own row is mentioned, not declared.
    let trailing = "1. [ ] - `p1` - first - `inst-x-one`; and more prose\n";
    assert!(declaration_counts(trailing).is_empty());
    assert!(mentioned_instructions(trailing).contains("inst-x-one"));
}

// ---------------------------------------------------------------------------
// Lints 7 and 8 — the §4 surfaces
// ---------------------------------------------------------------------------

/// The normative body of a slice — §2 through §5; §1 is context and §6 the
/// open items, neither a declaration site.
fn normative_body(text: &str) -> &str {
    let start = text.find("\n## 2. ").unwrap_or(0);
    let end = text.find("\n## 6. ").unwrap_or(text.len());
    &text[start..end.max(start)]
}

/// The blocks that declare a table, in the three shapes the set uses: a
/// bullet or numbered item whose first backticked identifier (after an
/// optional `` `pN` `` priority tag) is a `products_*` table (03, 06, 10); a
/// `###` heading naming the table, whose following blocks up to the next
/// heading are its columns (01 §4.1–4.4); and a bare paragraph opening with
/// the table's name. `design/10` declares its map in §3.1 and its §4 points
/// there, so declarations are read wherever the normative body puts them.
fn table_declarations(text: &str) -> Vec<(String, String)> {
    let ident = Regex::new(r"`([A-Za-z_][A-Za-z0-9_.-]*)`").unwrap();
    let priority = Regex::new(r"^p\d$").unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut open_heading: Option<String> = None;
    for block in normative_body(text).split("\n\n") {
        let trimmed = block.trim_start();
        let first = ident
            .captures_iter(block)
            .map(|c| c[1].to_owned())
            .find(|t| !priority.is_match(t));
        if trimmed.starts_with('#') {
            open_heading = first.filter(|t| t.starts_with("products_"));
            if let Some(table) = &open_heading {
                out.push((table.clone(), block.to_owned()));
            }
            continue;
        }
        match first {
            Some(t) if t.starts_with("products_") => out.push((t, block.to_owned())),
            _ => {
                if let Some(table) = &open_heading {
                    out.push((table.clone(), block.to_owned()));
                }
            }
        }
    }
    out
}

/// `(table, column)` pairs for every real-identity column a table
/// declaration carries.
fn identity_columns(text: &str) -> Vec<(String, String)> {
    let identity =
        Regex::new(r"`(principal_ref|principal_id|subject_id|operator_identity)`").unwrap();
    let mut out = Vec::new();
    for (table, block) in table_declarations(text) {
        for col in identity.captures_iter(&block) {
            out.push((table.clone(), col[1].to_owned()));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn identity_findings(columns: &[(String, String)]) -> Vec<String> {
    let tables: BTreeSet<&str> = columns.iter().map(|(t, _)| t.as_str()).collect();
    if tables == BTreeSet::from(["products_identity_ref"]) {
        return Vec::new();
    }
    vec![format!(
        "identity columns live in {tables:?}; exactly `products_identity_ref` may hold one"
    )]
}

#[test]
fn lint_7_exactly_one_table_materializes_an_operator_identity() {
    let mut columns = Vec::new();
    for (_, _, text) in slices() {
        columns.extend(identity_columns(&text));
    }
    let findings = identity_findings(&columns);
    assert!(findings.is_empty(), "lint 7: {findings:#?}");
    assert!(
        columns
            .iter()
            .any(|(t, c)| t == "products_identity_ref" && c == "principal_ref")
    );
}

#[test]
fn lint_7_fails_on_a_second_table_declaring_an_identity_column_and_ignores_actor_ref() {
    let one = identity_columns(
        "## 2. x\n\n1. [ ] - `p1` - `products_identity_ref` — `(tenant_id, actor_ref)` → `principal_ref`\n\n## 6. y",
    );
    assert!(identity_findings(&one).is_empty(), "{one:?}");
    let two = identity_columns(
        "## 2. x\n\n1. [ ] - `p1` - `products_identity_ref` — `principal_ref`\n\n2. [ ] - `p1` - `products_approval` — `submitter` as `principal_id`\n\n## 6. y",
    );
    let f = identity_findings(&two);
    assert_eq!(f.len(), 1, "{f:?}");
    let pseudonymous = identity_columns(
        "## 2. x\n\n1. [ ] - `p1` - `products_approval` — `actor_ref`, `approver_actor_ref`\n\n## 6. y",
    );
    assert!(
        pseudonymous.is_empty(),
        "a pseudonymous actor_ref is not an identity"
    );
    assert!(
        !identity_findings(&pseudonymous).is_empty(),
        "no identity table at all is a finding too"
    );
}

/// The backticked identifiers of every table declaration that name a
/// monetization model.
fn monetization_markers(text: &str) -> Vec<String> {
    let ident = Regex::new(r"`([A-Za-z_][A-Za-z0-9_-]*)`").unwrap();
    let banned = [
        "flat",
        "per-seat",
        "per_seat",
        "tiered",
        "volume",
        "hybrid",
        "commitment",
    ];
    let declarations = table_declarations(text);
    let scanned: String = declarations
        .iter()
        .map(|(_, b)| b.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut out = Vec::new();
    for m in ident.captures_iter(&scanned) {
        let token = m[1].to_lowercase();
        let parts: Vec<&str> = token.split(['_', '-']).collect();
        if banned
            .iter()
            .any(|b| token == *b || parts.contains(b) || (b.contains('-') && token.contains(b)))
        {
            out.push(m[1].to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn lint_8_no_storage_declaration_names_a_monetization_model() {
    let mut markers = Vec::new();
    let mut declarations = 0;
    for (n, _, text) in slices() {
        declarations += table_declarations(&text).len();
        for m in monetization_markers(&text) {
            markers.push(format!("{n}: `{m}`"));
        }
    }
    assert!(
        declarations >= 20,
        "{declarations} table declarations across the set"
    );
    assert!(markers.is_empty(), "lint 8: {markers:#?}");
}

#[test]
fn lint_8_fails_on_a_column_named_for_the_six_words_and_passes_usage() {
    let decl = |cols: &str| format!("## 4. x\n\n- `products_sku` — {cols}\n\n## 6. y");
    assert!(monetization_markers(&decl("`usage_type_ref`, `metering_unit`, `usage`")).is_empty());
    assert_eq!(
        monetization_markers(&decl("`tiered_price`")),
        vec!["tiered_price"]
    );
    assert_eq!(monetization_markers(&decl("`per-seat`")), vec!["per-seat"]);
    assert_eq!(
        monetization_markers(&decl("`commitment_months`, `flat`")),
        vec!["commitment_months", "flat"]
    );
    assert!(
        monetization_markers(&decl("`volumes_table_is_prose`")).is_empty(),
        "a word inside a word is not the word"
    );
    assert!(
        monetization_markers("## 4. x\n\nprose with `flat` beside no table\n\n## 6. y").is_empty(),
        "prose is not a declaration"
    );
}
