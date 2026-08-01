//! The artifact pricing's `FixtureGate` reads at publish time.
//!
//! Generated from the corpus by pricing's `regen_registry` example, committed,
//! and checked for freshness by that gear's `corpus_publish` test. The gate runs
//! inside a publish transaction and cannot run an oracle there, so it needs a
//! static answer.
//!
//! The generator is an example target of the **gear** rather than a binary of
//! either fixture crate because the two flags below are earned by two parties
//! that cannot see each other: the reference oracle lives in
//! `bss-fixtures-conformance`, pricing's validator lives in the gear, and the
//! harness is a dev-dependency of the gear so that no evaluator reaches it even
//! transitively. An example compiles with dev-dependencies, so it is the one
//! build in which both halves are visible.

use crate::kinds::ModelKind;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the three flags are the design: each is earned by a different party (oracle, pricing, rating) and collapsing them would hide which half is missing"
)]
pub struct VariantStatus {
    pub kind: ModelKind,
    /// The reference oracle reproduces every evaluation case for this kind.
    pub oracle: bool,
    /// Pricing's `PublishValidator` reproduces every publish case for this kind.
    /// A kind the corpus carries no publish case for earns nothing — absent
    /// coverage must never read as success.
    pub publish: bool,
    /// Rating's evaluator reproduces the same cases. False until rating exists;
    /// when it lands it must **agree** with the oracle, and disagreement reddens
    /// the corpus rather than overriding either side.
    pub rating: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default)]
    pub variants: Vec<VariantStatus>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

impl Registry {
    /// Reads the generated registry from disk.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if the file cannot be read or does not parse.
    pub fn load(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|source| RegistryError::Io {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| RegistryError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    /// What `FixtureGate` asks.
    ///
    /// Deliberately `oracle && publish`, not all three. The oracle is the
    /// executable §17.2 and stands in for Tariffs until Tariffs exists; making
    /// the gate wait for `rating` would block every publish at launch, since
    /// the rating gear has no code. An unregistered kind is never open.
    #[must_use]
    pub fn gate_open_for(&self, kind: ModelKind) -> bool {
        self.variants
            .iter()
            .any(|v| v.kind == kind && v.oracle && v.publish)
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod registry_tests;
