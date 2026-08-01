//! The joint-conformance publish gate (`inst-fx-gate`).
//!
//! Publish of a catalog `modelKind` is admissible only while a **green** joint
//! fixture pins what that kind means on the pricing <-> Rating seam. The corpus
//! lives in `gears/bss/fixtures`; its generated `registry.toml` is the artifact
//! this gate reads, because the gate runs inside a publish transaction and
//! cannot run an oracle there.
//!
//! The gear takes `bss-fixtures` with `default-features = false`. That surface
//! is `ModelKind` + `Registry` + `Registry::gate_open_for`, and the question it
//! answers is deliberately `oracle && publish`: the reference oracle stands in
//! for Tariffs until Tariffs exists, and requiring the `rating` half as well
//! would block every publish at launch. An unregistered kind is never open.
//!
//! **Fail-closed, in both directions.** There is no configuration value that
//! opens this gate, and a registry that cannot be read produces a *closed*
//! gate rather than a boot failure or a permissive default. The split is
//! deliberate: the read path this gear serves to Rating and Tariffs must keep
//! working when the fixtures artifact is missing from a deployment, while every
//! publish of every kind then fails per kind with `FIXTURE_MISSING`. A gate that
//! cannot read its registry has no basis on which to answer "green", and the
//! only safe answer it has left is "no".

use std::path::Path;

use bss_fixtures::{ModelKind, Registry};
use tracing::error;

use crate::domain::error::DomainError;

/// The publish-time conformance gate over a loaded [`Registry`].
///
/// Constructed once at gear init and read on every publish. Cheap to consult —
/// the registry is a handful of rows held in memory — which is what lets the
/// gate sit inside the publish transaction where it belongs.
#[derive(Debug, Clone)]
pub struct FixtureGate {
    registry: Registry,
}

impl FixtureGate {
    /// Load the generated registry from `path`.
    ///
    /// **Never fails.** An unreadable or unparseable registry yields
    /// [`FixtureGate::closed`] and is logged at `error!` with the path: the gear
    /// still boots and still serves reads, and every publish then fails closed
    /// with `FIXTURE_MISSING` naming the kind. Returning a `Result` here would
    /// invite a caller to treat the error as "gate unknown" and proceed, which
    /// is the one outcome this type exists to make unreachable.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match Registry::load(path) {
            Ok(registry) => Self { registry },
            Err(e) => {
                error!(
                    path = %path.display(),
                    error = %e,
                    "bss-pricing: conformance fixture registry unreadable; the publish gate is \
                     CLOSED for every model kind until it can be read"
                );
                Self::closed()
            }
        }
    }

    /// A gate that is closed for every kind.
    ///
    /// The empty registry is not a placeholder for "unknown": an unregistered
    /// kind is never open, so an empty variant list refuses everything by the
    /// same rule that refuses a kind whose fixtures are red.
    #[must_use]
    pub fn closed() -> Self {
        Self {
            registry: Registry::default(),
        }
    }

    /// The kinds this gate currently admits, in the corpus spelling.
    ///
    /// For the boot log, not for the publish path: an operator has to be able
    /// to read from the startup line which kinds are publishable, because the
    /// difference between "the corpus is green" and "the registry file was not
    /// deployed" is otherwise only visible one failed publish at a time.
    #[must_use]
    pub fn open_kinds(&self) -> Vec<&'static str> {
        ModelKind::ALL
            .into_iter()
            .filter(|kind| self.registry.gate_open_for(*kind))
            .map(wire_name)
            .collect()
    }

    /// Whether `kind` may be published.
    ///
    /// # Errors
    /// [`DomainError::FixtureMissing`] when no green joint fixture gates `kind`
    /// — the corpus has no row for it, its oracle half is unearned, or this
    /// gear's validator has not yet reproduced its publish cases. All three are
    /// the same refusal from the caller's side: nobody has agreed how this shape
    /// is evaluated, so it must not become consumer-visible.
    pub fn check(&self, kind: ModelKind) -> Result<(), DomainError> {
        if self.registry.gate_open_for(kind) {
            return Ok(());
        }
        Err(DomainError::FixtureMissing(format!(
            "model kind `{}` is not gated by a green joint conformance fixture",
            wire_name(kind)
        )))
    }
}

/// The corpus spelling of a kind.
///
/// The registry writes kinds in `snake_case`, and the refusal an operator reads
/// has to be greppable against that file — `PerUnit` is not a string that
/// appears in `registry.toml`. Exhaustive on purpose: a sixth kind cannot be
/// added to the enum without this match being extended, which is the same rule
/// the corpus enforces on itself for family coverage.
const fn wire_name(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Flat => "flat",
        ModelKind::PerUnit => "per_unit",
        ModelKind::Graduated => "graduated",
        ModelKind::Volume => "volume",
        ModelKind::Package => "package",
    }
}

#[cfg(test)]
#[path = "fixture_gate_tests.rs"]
mod fixture_gate_tests;
