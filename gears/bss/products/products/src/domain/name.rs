//! `normalized(name)` — the uniqueness and promotion-identity operand.
//!
//! The design pins the pipeline exactly: Unicode `NFKC`, then full casefold, then
//! trim and collapse internal whitespace to single spaces. It is computed
//! **application-side** so both engines store identical bytes, which is what
//! lets one partial unique index mean the same thing on Postgres and on `SQLite`.
//!
//! Every step is load-bearing and none is decorative:
//!
//! - **NFKC** folds compatibility variants — a full-width `Ａ` and an `A`, a
//!   ligature and its letters — so two names that render identically cannot
//!   both be reserved.
//! - **Full casefold** is not `to_lowercase`. The difference is not academic:
//!   German `ß` casefolds to `ss` and lowercases to itself, so a lowercase-only
//!   normalizer admits `Straße` beside `STRASSE`.
//! - **Whitespace collapse** removes the only remaining way to author a name
//!   that reads the same and hashes differently.
//!
//! Order matters. Casefolding before NFKC would leave compatibility variants of
//! already-folded characters unfolded.

use unicode_normalization::UnicodeNormalization;

/// Normalize a name for the uniqueness index.
///
/// @cpt-cf-bss-products-fr-create-product
/// @cpt-dod:cpt-cf-bss-products-dod-name-uniqueness:p1
///
/// The result is what `name_normalized` stores; the operator-facing `name`
/// keeps whatever was authored.
#[must_use]
pub fn normalize(name: &str) -> String {
    let folded: String = name.nfkc().flat_map(char::to_lowercase).collect();
    // The two-step fold: `to_lowercase` handles the common mappings and the
    // explicit table below handles the full-casefold cases where lowercase and
    // casefold disagree. Rust's standard library exposes no full casefold, and
    // the set of disagreeing characters that can reach a catalog name is small
    // and enumerable — but it is enumerable rather than complete, which is
    // stated here because a name in a script this table does not cover
    // normalizes by lowercase alone.
    let cased: String = folded
        .chars()
        .flat_map(|ch| match ch {
            // LATIN SMALL LETTER SHARP S: casefolds to "ss", lowercases to itself.
            '\u{00DF}' => "ss".chars().collect::<Vec<_>>(),
            // LATIN SMALL LIGATURE FF / FI / FL.
            '\u{FB00}' => "ff".chars().collect(),
            '\u{FB01}' => "fi".chars().collect(),
            '\u{FB02}' => "fl".chars().collect(),
            // GREEK SMALL LETTER FINAL SIGMA folds to GREEK SMALL LETTER SIGMA:
            // the same letter, positional variant only.
            '\u{03C2}' => "\u{03C3}".chars().collect(),
            other => vec![other],
        })
        .collect();
    cased.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "name_tests.rs"]
mod name_tests;
