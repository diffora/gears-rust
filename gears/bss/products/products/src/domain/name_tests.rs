//! The normalization is a cross-engine byte contract, so the tests assert the
//! bytes rather than a predicate over them.
//!
//! Non-ASCII literals are written out rather than escaped here, and the
//! allowance is deliberate: every case in this file exists to show that two
//! spellings of the same name must normalize together, and a case rendered as
//! `\u{00DF}` cannot show that to a reader. The production table in
//! `super` uses escapes, where precision matters more than legibility.
#![allow(clippy::non_ascii_literal)]

use super::normalize;

#[test]
fn whitespace_is_trimmed_and_collapsed() {
    assert_eq!(normalize("  Acme   Widget \t Pro \n"), "acme widget pro");
}

#[test]
fn case_is_folded() {
    assert_eq!(normalize("ACME Widget"), normalize("acme widget"));
}

#[test]
fn full_casefold_differs_from_lowercase_and_this_is_the_case_that_proves_it() {
    // `to_lowercase` leaves `ß` alone; full casefold makes it `ss`. A
    // lowercase-only normalizer would admit both of these as distinct names.
    assert_eq!(normalize("Straße"), "strasse");
    assert_eq!(normalize("STRASSE"), "strasse");
    assert_eq!(normalize("Straße"), normalize("STRASSE"));
}

#[test]
fn nfkc_folds_compatibility_variants() {
    // Full-width Latin renders identically to ASCII in most catalogs.
    assert_eq!(normalize("ＡＣＭＥ"), "acme");
    // A ligature and its letters must not both be reservable.
    assert_eq!(normalize("ﬁle"), normalize("file"));
}

#[test]
fn the_empty_and_whitespace_only_cases_collapse_to_empty() {
    assert_eq!(normalize(""), "");
    assert_eq!(normalize("   \t\n "), "");
}

#[test]
fn normalization_is_idempotent() {
    for input in ["  Acme   Widget  ", "Straße", "ＡＣＭＥ", "ﬁle", "ΣΊΣΥΦΟΣ"] {
        let once = normalize(input);
        assert_eq!(normalize(&once), once, "second pass moved {input:?}");
    }
}

#[test]
fn a_final_sigma_folds_to_the_medial_form() {
    // Greek final sigma and medial sigma are the same letter; a name ending in
    // one must not be reservable beside the other.
    assert_eq!(normalize("ΣΊΣΥΦΟΣ"), normalize("σίσυφος"));
}
