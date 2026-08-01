//! ISO 4217 money primitives: the validated [`CurrencyCode`], its minor-unit
//! scale, and the non-negative [`MinorAmount`].
//!
//! **This gear computes no charge.** It authors, validates and freezes price
//! rows; the arithmetic that turns a row into an amount owed lives in Tariffs
//! (evaluation) and Billing (invoicing). So there is deliberately **no add,
//! subtract, multiply, allocate or round here** — only the checks a publish
//! needs. A helper that added two `MinorAmount`s would invite exactly the
//! second implementation of charge arithmetic the design set puts downstream,
//! and the sibling ledger already owns the rounding and allocation idiom for
//! the money that is actually posted.
//!
//! Amounts are integer minor units (`amount_minor`), never a float and never a
//! decimal type. The per-currency **scale** is the ISO 4217 minor unit: there is
//! no flat two-decimal rule — JPY takes 0 and BHD takes 3 — which is why
//! `PRECISION_EXCEEDED` is a first-class publish rejection rather than a
//! rounding decision taken quietly at the boundary.

use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::error::DomainError;

/// The minor unit assumed for any code outside the two tables below.
///
/// Named rather than inlined so the fallback is a stated policy: an unlisted
/// code is treated as two-decimal, it is never rejected for being unlisted and
/// it never resolves to "unknown".
pub const DEFAULT_MINOR_UNIT: u32 = 2;

/// ISO 4217 codes with **no** minor unit.
///
/// A **launch subset**, deliberately: it carries the zero-decimal codes we are
/// confident about and nothing else, because a half-remembered table is worse
/// than a named fallback. Anything absent resolves to [`DEFAULT_MINOR_UNIT`].
/// The four-decimal fund codes (CLF, UYW) are absent on purpose — they are unit
/// of account codes, not currencies anything is sold in, and admitting one
/// would put a scale above 3 into the key.
const ZERO_DECIMAL_CODES: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "VND", "VUV",
    "XAF", "XOF", "XPF",
];

/// ISO 4217 codes with a **three**-decimal minor unit. Same launch-subset rule
/// as [`ZERO_DECIMAL_CODES`].
const THREE_DECIMAL_CODES: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];

/// A validated ISO 4217 alpha-3 currency code, held uppercase.
///
/// Normalization is load-bearing, not cosmetic: `currency` is a **scope-key
/// axis**, so `usd` and `USD` must be one key or the duplicate-key index stops
/// seeing duplicates. The constructor therefore accepts either spelling and
/// stores exactly one of them.
///
/// Membership of the real ISO register is not checked here — that is a
/// taxonomy question the tenant currency configuration answers (Slice 4). What
/// this type guarantees is the *shape* every downstream axis comparison relies
/// on: three ASCII letters, uppercase.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Validate and normalize an authored currency code.
    ///
    /// # Errors
    ///
    /// [`DomainError::CurrencyInvalid`] when `raw` is not three ASCII letters.
    pub fn new(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.len() != 3 || !trimmed.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(DomainError::CurrencyInvalid(raw.to_owned()));
        }
        Ok(Self(trimmed.to_ascii_uppercase()))
    }

    /// The normalized uppercase code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The ISO 4217 minor unit: the number of decimal places one unit of this
    /// currency is subdivided into, and therefore the maximum precision a price
    /// on it can express.
    #[must_use]
    pub fn minor_unit(&self) -> u32 {
        let code = self.0.as_str();
        if ZERO_DECIMAL_CODES.contains(&code) {
            0
        } else if THREE_DECIMAL_CODES.contains(&code) {
            3
        } else {
            DEFAULT_MINOR_UNIT
        }
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An amount in integer **minor units**, validated non-negative.
///
/// Non-negativity is a checked property rather than an assumption: typed credit
/// rows are deliberately Future scope, so a negative price is an authoring
/// mistake and must be refused where it enters, not carried until some
/// downstream sum looks wrong.
///
/// The type deliberately carries **no currency**: the currency is a scope-key
/// axis, so pairing one into the amount would create a second place to disagree
/// about which currency a row is priced in.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MinorAmount(i64);

impl MinorAmount {
    /// Validate an authored minor-unit amount.
    ///
    /// # Errors
    ///
    /// [`DomainError::AmountNegative`] when `units` is below zero. Zero is
    /// valid — a free trial phase and a zero-rated usage row are both authored
    /// as `0`, not as an absent amount.
    pub fn new(units: i64) -> Result<Self, DomainError> {
        if units < 0 {
            return Err(DomainError::AmountNegative(units.to_string()));
        }
        Ok(Self(units))
    }

    /// The underlying minor-unit count.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for MinorAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Is an amount authored with `declared_scale` fraction digits expressible in
/// `currency`?
///
/// The predicate is over the **declared** precision, not over the value: a
/// price authored as `100.00` on JPY is not expressible, even though `100` is,
/// because the author has stated a two-decimal price on a currency that has no
/// decimals. Silently accepting it would mean the read model froze a scale the
/// currency cannot represent, and every consumer would have to re-derive which
/// digits were real.
#[must_use]
pub fn is_expressible(currency: &CurrencyCode, declared_scale: u32) -> bool {
    declared_scale <= currency.minor_unit()
}

/// The publish-time form of [`is_expressible`].
///
/// # Errors
///
/// [`DomainError::PrecisionExceeded`] when `declared_scale` is above the
/// currency's ISO 4217 minor unit.
pub fn check_scale(currency: &CurrencyCode, declared_scale: u32) -> Result<(), DomainError> {
    if is_expressible(currency, declared_scale) {
        return Ok(());
    }
    Err(DomainError::PrecisionExceeded(format!(
        "{currency} has {} decimal(s), amount declares {declared_scale}",
        currency.minor_unit()
    )))
}

/// The number of fraction digits an authored decimal literal declares.
///
/// This is the shape an authoring surface submits (`"1.50"`), so the scale
/// check has to be able to start from it rather than from an already-split
/// `(value, scale)` pair.
///
/// # Errors
///
/// [`DomainError::InvalidRequest`] when `literal` is not a decimal number: an
/// empty part, a second decimal point, or any non-digit outside the leading
/// sign. The sign itself is accepted here and judged by [`MinorAmount::new`] —
/// one rejection per problem, so an author is not told about precision when the
/// real fault is a negative price.
pub fn fraction_digits(literal: &str) -> Result<u32, DomainError> {
    let malformed = || DomainError::InvalidRequest(format!("malformed decimal literal: {literal}"));
    let body = literal.strip_prefix('-').unwrap_or(literal);
    let mut parts = body.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some() || whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    let Some(fraction) = fraction else {
        return Ok(0);
    };
    if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    u32::try_from(fraction.len()).map_err(|_| malformed())
}

/// Is an authored decimal literal expressible in `currency`?
///
/// # Errors
///
/// [`DomainError::InvalidRequest`] when the literal is malformed;
/// [`DomainError::PrecisionExceeded`] when it declares more precision than the
/// currency's minor unit.
pub fn check_decimal(currency: &CurrencyCode, literal: &str) -> Result<(), DomainError> {
    check_scale(currency, fraction_digits(literal)?)
}

#[cfg(test)]
#[path = "money_tests.rs"]
mod money_tests;
