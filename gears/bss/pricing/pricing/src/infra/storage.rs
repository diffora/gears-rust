//! Persistence error and migration frames for the pricing rebuild.

pub mod migrations;

use crate::infra::error_mapping::DomainError;

/// No repository errors are constructible until repositories return in phase 2c.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RepoError {}

/// Convert the repository error vocabulary to the canonical mapping input.
#[must_use]
pub fn repo_failure(error: &RepoError) -> DomainError {
    match *error {}
}
