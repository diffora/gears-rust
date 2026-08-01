//! The BSS joint golden conformance fixture corpus — loader and registry.
//!
//! This crate deliberately contains no charge arithmetic. The catalog publishes
//! structure and never computes a charge, so an evaluator must not reach it even
//! transitively; the arithmetic lives in `bss-fixtures-conformance`, which is a
//! dev-dependency only.
//!
//! Design: `docs/superpowers/specs/2026-08-01-bss-joint-fixture-corpus-design.md`.

pub mod kinds;
pub mod registry;

pub use kinds::ModelKind;
pub use registry::{Registry, RegistryError, VariantStatus};

// Everything below reads the corpus from disk. A gear never needs it: the
// publish gate asks the registry whether a kind is green and nothing more, so
// the loader, the case model and the integrity checks stay behind a feature
// rather than riding into production on a dependency that only wanted an answer.
#[cfg(feature = "corpus")]
pub mod corpus;
#[cfg(feature = "corpus")]
pub mod integrity;
#[cfg(feature = "corpus")]
pub mod model;

#[cfg(feature = "corpus")]
pub use corpus::{Corpus, CorpusError, FamilyMeta, GateRole};
#[cfg(feature = "corpus")]
pub use integrity::{IntegrityViolation, check_integrity, check_kind_coverage};
#[cfg(feature = "corpus")]
pub use model::{
    AggregationFunction, AggregationGranularity, Assertion, Band, BandTop, Case, CaseHeader,
    CaseKind, ChargeExpect, EvaluationCase, Expect, Family, FoldExpect, GaugeSample, Given,
    IncludedAllowance, ProrationBasis, PublishAssertion, PublishCase, PublishVerdict,
    ReservationFlavor, RolloverPolicy, Runtime, Snapshot, UnitsExpect,
};
