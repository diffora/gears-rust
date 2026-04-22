// Created: 2026-04-21 by Constructor Tech
// Updated: 2026-04-21 by Constructor Tech
#![allow(dead_code)]

use std::fmt;

#[derive(Debug)]
struct DatabaseError(String);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DatabaseError {}

#[derive(Debug)]
struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

// Intentionally NOT `impl Error for ConversionRejected` — this test exercises
// the path where the source (`DatabaseError`) is the Error-implementing gate
// trigger, not the associated `Error` type. That way the lint's firing here
// depends only on the source-type match, not on the assoc-type gate.
#[derive(Debug)]
struct ConversionRejected;

// `TryFrom` impls are also covered.
impl TryFrom<DatabaseError> for AppError {
    type Error = ConversionRejected;

    fn try_from(e: DatabaseError) -> Result<Self, Self::Error> {
        // Should trigger DE1302 - to_string
        Ok(AppError(e.to_string()))
    }
}

fn main() {}
