//! Tenant dimension vocabulary validation.
use super::RuleError;
use std::collections::BTreeSet;

fn key_code(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn value_code(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
/// Validate a registry entry. Removal of a used value is a door-level rule.
#[must_use]
pub fn validate(key: &str, values: &[String]) -> Vec<RuleError> {
    let mut errors = Vec::new();
    if !key_code(key.trim()) {
        errors.push(RuleError::new("DIM_KEY_INVALID"));
    }
    if values.len() < 2 {
        errors.push(RuleError::new("DIM_VALUES_FEW"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        if !value_code(value) {
            errors.push(RuleError::new("DIM_VALUE_INVALID"));
        }
        if !seen.insert(value) {
            errors.push(RuleError::new("DIM_VALUE_DUPLICATE"));
        }
    }
    errors
}
#[cfg(test)]
#[path = "dimension_tests.rs"]
mod tests;
