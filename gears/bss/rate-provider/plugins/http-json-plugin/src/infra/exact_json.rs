//! A JSON tree that keeps every number as the exact token the provider wrote.
//!
//! `serde_json::Value` holds a number as `f64`, `i64` or `u64` unless the
//! `arbitrary_precision` feature is on, and that feature changes number handling
//! for every crate in the binary. This tree is built through `RawValue` (the
//! additive `raw_value` feature), so a quote such as
//! `1.123456789123456789123456789` reaches `parse_rate` digit for digit.

use std::collections::BTreeMap;

use serde_json::value::RawValue;

/// One JSON value; a number is its exact source token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExactJson {
    Null,
    Bool(bool),
    /// The number token exactly as written (`0.92`, `1e-7`, `-3`).
    Number(String),
    String(String),
    Array(Vec<ExactJson>),
    Object(BTreeMap<String, ExactJson>),
}

impl ExactJson {
    /// Parse a JSON document, keeping number tokens exact.
    ///
    /// # Errors
    /// The `serde_json` error for malformed JSON.
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let raw: &RawValue = serde_json::from_slice(bytes)?;
        Self::from_raw(raw)
    }

    fn from_raw(raw: &RawValue) -> Result<Self, serde_json::Error> {
        let text = raw.get();
        match text.as_bytes().first() {
            Some(b'{') => {
                let members: BTreeMap<String, &RawValue> = serde_json::from_str(text)?;
                members
                    .into_iter()
                    .map(|(key, value)| Ok((key, Self::from_raw(value)?)))
                    .collect::<Result<_, _>>()
                    .map(Self::Object)
            }
            Some(b'[') => {
                let items: Vec<&RawValue> = serde_json::from_str(text)?;
                items
                    .into_iter()
                    .map(Self::from_raw)
                    .collect::<Result<_, _>>()
                    .map(Self::Array)
            }
            Some(b'"') => serde_json::from_str(text).map(Self::String),
            Some(b't' | b'f') => serde_json::from_str(text).map(Self::Bool),
            Some(b'n') => Ok(Self::Null),
            // `RawValue` holds only valid JSON, so what remains is a number token.
            _ => Ok(Self::Number(text.to_owned())),
        }
    }

    /// The member `key` of an object.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(members) => members.get(key),
            _ => None,
        }
    }

    /// The text of a string value.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The members of an object.
    #[must_use]
    pub fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        match self {
            Self::Object(members) => Some(members),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "exact_json_tests.rs"]
mod tests;
