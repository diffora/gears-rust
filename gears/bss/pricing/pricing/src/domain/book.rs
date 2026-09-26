//! Book content and half-open sale validity.
use super::RuleError;
use time::Date;

#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub name: String,
    pub currency: String,
    pub valid_from: Option<Date>,
    pub valid_until: Option<Date>,
}
/// Validate content. Code uniqueness belongs to the repository.
#[must_use]
pub fn validate(book: &Book) -> Vec<RuleError> {
    let mut errors = Vec::new();
    if book.name.trim().is_empty() {
        errors.push(RuleError::new("BOOK_NAME_REQUIRED"));
    }
    if book.currency.len() != 3 || !book.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        errors.push(RuleError::new("BOOK_CURRENCY_INVALID"));
    }
    if matches!((book.valid_from,book.valid_until),(Some(a),Some(b)) if a>=b) {
        errors.push(RuleError::new("BOOK_VALIDITY_INVALID"));
    }
    errors
}
/// Whether sales from this book are allowed on a date.
#[must_use]
pub fn valid_on(book: &Book, date: Date) -> bool {
    book.valid_from.is_none_or(|start| date >= start)
        && book.valid_until.is_none_or(|end| date < end)
}
/// Fractional digits of an ISO 4217 currency; two unless the standard says otherwise.
#[must_use]
pub fn minor_digits(currency: &str) -> u32 {
    match currency {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        "CLF" | "UYW" => 4,
        _ => 2,
    }
}
#[cfg(test)]
#[path = "book_tests.rs"]
mod tests;
