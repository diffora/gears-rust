//! Lint 9 — the obligation×pin coupling (`design/12` §3.2 item 9; P-D-12,
//! P-D-43 as P-D-63 amended it, P-D-65), executable.
//!
//! Both sides ship, so the lint runs as this crate's own test rather than
//! waiting for the nine-lint CI job (`dod-lint-gate`, owned outside the
//! gear): the `SchemaPin` is `schema-pin.toml` beside this crate's
//! manifest, and the `ObligationRegister` is §2.2's table in
//! `design/12-consumer-contracts.md`.
//!
//! It lives under `src/` rather than in `tests/` because the traceability
//! scanner's registered code roots for a BSS gear are `src`, `tests`,
//! `<crate>/src`, `<crate>/tests` and `<crate>-sdk/src` — **there is no
//! `<crate>-sdk/tests` root**, so a marker in this crate's `tests/`
//! directory is invisible to the gate and the `DoD` can never be satisfied
//! from there. `#[cfg(test)]` keeps it out of every shipped build. A pin member added without a register
//! row naming it, or a register operand naming a field the pin does not
//! carry, fails here **in the change that introduced it**.
//!
//! # The grammar this lint reads (P-D-63's letter)
//!
//! The `Operand` cell is the lint's ONLY input, and it reads **tokens
//! only**: a backticked identifier is a token, and any prose beside the
//! tokens is ignored — a cell is never judged by being read. Three marker
//! forms leave the field-comparison population (P-D-65's narrowing):
//!
//! - `` `X` (surface) `` — one token; the annotation names the surface and
//!   couples to the pin's `surface` entries in both directions;
//! - `` `X` payload `` — one token; an event-payload operand, outside the
//!   pin by construction (the pin covers the entity surface);
//! - `none in v1` — the whole CELL is outside the coupling population by
//!   the marker rule, so nothing in it (its explanatory prose included) is
//!   a token.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-lint-pin-coupling:p1

// The test posture the sibling gears' own suites take: a lint that cannot
// read its input must panic loudly, not return an error nothing collects.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;

/// One pin member, as the lint reads it: the joint name, the kind, and an
/// optional recorded exclusion reason (a field the register deliberately
/// does not name — none exists today, and the key is read so that adding
/// one is a recorded decision rather than a lint suppression).
struct PinMember {
    name: String,
    kind: String,
    exclusion_reason: Option<String>,
}

fn read_pin() -> Vec<PinMember> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema-pin.toml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the SchemaPin must be readable at {}: {e}", path.display()));
    let value: toml::Value = raw.parse().expect("schema-pin.toml must parse as TOML");
    value
        .get("member")
        .and_then(toml::Value::as_array)
        .expect("the pin carries [[member]] entries")
        .iter()
        .map(|member| PinMember {
            name: member
                .get("name")
                .and_then(toml::Value::as_str)
                .expect("every member carries a name")
                .to_owned(),
            kind: member
                .get("kind")
                .and_then(toml::Value::as_str)
                .expect("every member carries a kind")
                .to_owned(),
            exclusion_reason: member
                .get("exclusion-reason")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
        })
        .collect()
}

/// The register's `Operand` cells, in row order — extracted from §2.2's
/// table by its own header, so a second table in the document cannot be
/// mistaken for the register.
fn read_operand_cells() -> Vec<String> {
    read_register_rows().into_iter().map(|row| row.0).collect()
}

/// One register row's `(Operand, Status)` pair.
type RegisterRow = (String, String);

/// The register's rows, in order — the roster **and** its status column, so
/// the census `dod-obligation-register` states in prose can be asserted
/// rather than read.
fn read_register_rows() -> Vec<RegisterRow> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/design/12-consumer-contracts.md")
        .canonicalize()
        .expect("the design document sits beside this crate");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("design/12 must be readable at {}: {e}", path.display()));

    let mut cells: Vec<RegisterRow> = Vec::new();
    let mut in_table = false;
    for line in raw.lines() {
        if line.starts_with('|') && line.contains("Operand (what its guard reads") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if !line.starts_with('|') {
            break; // the table ended
        }
        let columns: Vec<&str> = line.trim_matches('|').split('|').collect();
        // | Obligation | Owing consumer | Source | Operand | Status |
        if columns.len() < 5 || columns[0].trim().starts_with('-') {
            continue; // the separator row
        }
        cells.push((columns[3].trim().to_owned(), columns[4].trim().to_owned()));
    }
    assert!(
        !cells.is_empty(),
        "the ObligationRegister's table must be found by its Operand header"
    );
    cells
}

/// One cell's tokens, per the grammar: `(field_tokens, surface_annotations,
/// payload_seen)`. A `none in v1` cell answers empty everything — the
/// marker rule takes the whole cell out of the population.
fn tokens_of(cell: &str) -> (Vec<String>, Vec<String>, bool) {
    if cell.contains("none in v1") {
        return (Vec::new(), Vec::new(), false);
    }
    let mut fields = Vec::new();
    let mut surfaces = Vec::new();
    let mut payload = false;
    let mut rest = cell;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let identifier = &after[..close];
        let tail = &after[close + 1..];
        if let Some(stripped) = tail.strip_prefix(" (surface)") {
            surfaces.push(identifier.to_owned());
            rest = stripped;
        } else if let Some(stripped) = tail.strip_prefix(" payload") {
            payload = true;
            rest = stripped;
        } else {
            fields.push(identifier.to_owned());
            rest = tail;
        }
    }
    (fields, surfaces, payload)
}

/// The coupling, both directions (P-D-12): every field token appears in the
/// pin, and every pinned field is an obligation operand or carries a
/// recorded exclusion reason. Surface tokens couple to the pin's `surface`
/// entries in both directions (P-D-65); `payload` couples to nothing.
#[test]
fn every_register_operand_and_every_pin_member_couple() {
    let pin = read_pin();
    let cells = read_operand_cells();

    let pin_fields: BTreeSet<&str> = pin
        .iter()
        .filter(|m| m.kind == "field")
        .map(|m| m.name.as_str())
        .collect();
    let pin_surfaces: BTreeSet<&str> = pin
        .iter()
        .filter(|m| m.kind == "surface")
        .map(|m| m.name.as_str())
        .collect();
    assert!(
        !pin_fields.is_empty(),
        "an empty pin would make every assertion below vacuous"
    );

    let mut named_fields: BTreeSet<String> = BTreeSet::new();
    let mut named_surfaces: BTreeSet<String> = BTreeSet::new();
    for cell in &cells {
        let (fields, surfaces, _payload) = tokens_of(cell);
        for token in fields {
            assert!(
                pin_fields.contains(token.as_str()),
                "operand cell {cell:?} names the field {token:?}, which the SchemaPin does not \
                 carry — either pin it or record why it is outside"
            );
            named_fields.insert(token);
        }
        for annotation in surfaces {
            assert!(
                pin_surfaces.contains(annotation.as_str()),
                "operand cell {cell:?} carries a (surface) token for {annotation:?}, which the \
                 SchemaPin does not pin as a surface"
            );
            named_surfaces.insert(annotation);
        }
    }

    // The other direction: a pinned member no obligation reads is drift the
    // FR could not see — the reason the coupling exists at all.
    for member in &pin {
        let named = match member.kind.as_str() {
            "field" => named_fields.contains(&member.name),
            "surface" => named_surfaces.contains(&member.name),
            other => panic!(
                "pin member {:?} carries unknown kind {other:?}",
                member.name
            ),
        };
        assert!(
            named || member.exclusion_reason.is_some(),
            "the pin carries {:?} ({}) but no ObligationRegister operand names it and it \
             records no exclusion reason — PlanTier's own history is why this direction is \
             checked",
            member.name,
            member.kind
        );
    }

    // Non-vacuousness: the current register names every pinned field, so a
    // parse regression that silently empties the token stream must fail
    // loudly rather than pass an empty check.
    assert_eq!(
        named_fields.len(),
        pin_fields.len(),
        "every pinned field is named by the register today; a shrink here is a parse \
         regression or a real decoupling, and both deserve a look"
    );
}

/// The register's own census, executable (`dod-obligation-register`).
///
/// `design/12` §2.2 is the **normative roster**, and the FEATURE states its
/// shape in prose: fourteen rows, of which twelve are `owed` in some form,
/// one is `assertable now` and one is `deferred with post-v1 EOL`. Prose
/// counts drift — this crate's own history has several — so the roster is
/// asserted here, beside the lint that reads the same table. A row added,
/// removed, or moved between statuses fails **in the change that moves it**,
/// which is what makes the register a roster rather than a list.
///
/// It also re-asserts P-D-63's outcome: after the four `+` cells were
/// normalized, **all fourteen cells parse** — so no cell may reintroduce a
/// separator the grammar refuses.
///
/// @cpt-dod:cpt-cf-bss-products-dod-obligation-register:p1
#[test]
fn the_obligation_register_carries_its_stated_roster() {
    let rows = read_register_rows();
    assert_eq!(
        rows.len(),
        14,
        "the ObligationRegister is fourteen rows; design/12 section 6 row 15 still says \
         thirteen and that is recorded against its owner, not repaired here"
    );

    let mut owed = 0_usize;
    let mut assertable = 0_usize;
    let mut deferred = 0_usize;
    for (cell, status) in &rows {
        // P-D-63: `+` is not admitted to the grammar. A cell that reintroduces
        // it is a decoupling the lint above would not necessarily see, because
        // what `+` joined was a field and a PHRASE — the phrase's pin members
        // simply vanish from the token stream.
        assert!(
            !cell.contains('+'),
            "operand cell {cell:?} carries `+`, which P-D-63 normalized out: rewrite it to \
             comma-separated pin tokens"
        );
        // The status is classified by its LEADING class, never by a substring
        // anywhere in the cell. One row's prose quotes pricing's own document
        // calling a binding "deferred" while the row itself is `owed`, so a
        // `contains` test reads prose and miscounts — which is the very defect
        // the `Operand` column exists to avoid, committed one column over.
        let leading = status.replace('*', "").trim().to_ascii_lowercase();
        if leading.starts_with("assertable now") {
            assertable += 1;
        } else if leading.starts_with("deferred") {
            deferred += 1;
        } else {
            assert!(
                leading.starts_with("owed"),
                "row {cell:?} carries status {status:?}, whose leading class is none of the \
                 register's three"
            );
            owed += 1;
        }
    }

    assert_eq!(
        (owed, assertable, deferred),
        (12, 1, 1),
        "the register's stated distribution is twelve owed, one assertable now, one deferred"
    );
}
