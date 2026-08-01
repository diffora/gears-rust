//! The joint-conformance publish gate (`inst-fx-gate`).
//!
//! Publish of a catalog row is admissible only while a **green** joint fixture
//! pins what that row means on the pricing <-> Rating seam. The corpus lives in
//! `gears/bss/fixtures`; its generated `registry.toml` is the artifact this gate
//! reads, because the gate runs inside a publish transaction and cannot run an
//! oracle there.
//!
//! ## The gate is asked about a row, not about a kind
//!
//! A row needs **every fixture its shape requires**, and the design set names
//! four of them as registry *variants* (S3 design 6: the registry is keyed
//! `model_kind` / `variant`):
//!
//! | The row is ... | and it additionally needs |
//! |---|---|
//! | anything | its `modelKind` variant (`inst-fx-gate`, `inst-fx-kinds`) |
//! | non-`sum` | the `level_aggregation` variant (`inst-la-fixture`, D-44) |
//! | a tiered **usage** kind | the `supersession_continuity` variant (D-22) |
//! | reserved | the `reserved` variant (`inst-fx-kinds`, Slice 10) |
//!
//! Asked about a bare `modelKind` the last three say nothing: a `peak` row would
//! publish on the strength of a `sum` row's fixture, and the level-aggregation,
//! supersession-continuity and reservation variants would gate nothing at all.
//! [`required_variants`] is where a row's variant set is decided, and it is the
//! whole content of that fix.
//!
//! The gear takes `bss-fixtures` with `default-features = false`. That surface
//! is `ModelKind` + `Variant` + `Registry` + `Registry::gate_open_for`, and the
//! question it answers per variant is deliberately `oracle && publish`: the
//! reference oracle stands in for Tariffs until Tariffs exists, and requiring
//! the `rating` half as well would block every publish at launch. An
//! unregistered `(kind, variant)` pair is never open.
//!
//! ## This is not `pricing_conformance_fixture_registry`
//!
//! S3 design 6 also specifies a **gear-side table**,
//! `pricing_conformance_fixture_registry` (`model_kind` / `variant`,
//! `fixture_ref`, `status`), tenant-global and read-only to every API path,
//! populated by the `FixtureRegistrySync` background task. That table is Slice
//! 3's and **does not exist yet**; nothing here builds it, and `fixture_ref` /
//! `status` appear nowhere in this gear.
//!
//! The two must not blur together. The corpus registry read here is a property
//! of the *gear build*: it ships in the artifact, it is regenerated from the
//! case files, and it carries no `fixture_ref` because the fixture it points at
//! is the corpus itself. When the table lands, this gate reads the table and the
//! generated file becomes what `FixtureRegistrySync` reconciles it *against* —
//! the variant vocabulary below is the same one, which is what will let the
//! swap be a swap rather than a second reading.
//!
//! **Fail-closed, in both directions.** There is no configuration value that
//! opens this gate, and a registry that cannot be read produces a *closed*
//! gate rather than a boot failure or a permissive default. The split is
//! deliberate: the read path this gear serves to Rating and Tariffs must keep
//! working when the fixtures artifact is missing from a deployment, while every
//! publish of every row then fails with `FIXTURE_MISSING`. A gate that cannot
//! read its registry has no basis on which to answer "green", and the only safe
//! answer it has left is "no".

use std::path::Path;

use bss_fixtures::{ModelKind, Registry, Variant};
use tracing::error;

use crate::domain::error::DomainError;
use crate::domain::price_row::{PriceRow, model_kind_wire};

/// Whether the publishing row carries a reservation — **the named hole**.
///
/// `reservationFlavor` and `reservedRateMinor` are Slice-10 fields and
/// [`PriceRow`] is the Slice-3 shape, so this gear cannot yet look at a row and
/// tell. That fact is a parameter rather than an omission: the reservation
/// variant is a stated gate (`inst-fx-kinds` — "the reservation variant of a
/// usage row requires its own fixture (Slice 10 registers it into this gate)"),
/// and a gate that silently never asked would be indistinguishable from a gate
/// that asked and was always answered "no".
///
/// So the caller must say, at every call site, which of the two it means. When
/// Slice 10 lands, [`Reservation::of_slice3_row`] starts reading the row's own
/// flavor and every caller keeps compiling — see its docs for the one line that
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reservation {
    /// The row reserves nothing.
    #[default]
    Unreserved,
    /// The row is reserved, and therefore needs [`Variant::Reserved`].
    Reserved,
}

impl Reservation {
    /// What a Slice-3 row can say about itself: **nothing**.
    ///
    /// A constant, and that is the hole stated in code. [`PriceRow`] carries
    /// neither `reservedRateMinor` nor `reservationFlavor` — the corpus's own
    /// `reserved` snapshots are declined by the publish validator for exactly
    /// that reason (`consumption-on-level-rejected` is `declined_until`
    /// Slice 10) — so there is no field to read and inventing one would be the
    /// gear guessing at an axis it does not own.
    ///
    /// The day Slice 10 adds the pair to [`PriceRow`], this body becomes a match
    /// on `row.reservation_flavor` and the gate starts demanding
    /// [`Variant::Reserved`] of reserved rows without any call site moving.
    /// Until then a caller that *knows* it holds a reserved row passes
    /// [`Reservation::Reserved`] explicitly, and the gate answers correctly
    /// today — which is what `a_reserved_row_is_gated_on_its_own_variant` in the
    /// sibling test file proves.
    #[must_use]
    pub const fn of_slice3_row(_row: &PriceRow) -> Self {
        Self::Unreserved
    }
}

/// Every registry variant a publishing row must be green in, in the order the
/// refusal reports them.
///
/// **This function is the mapping.** Each clause is one instruction of the
/// design set:
///
/// - [`Variant::ModelKind`] unconditionally — `inst-fx-gate` blocks publish of
///   any `modelKind` lacking a green joint fixture, and `inst-fx-kinds` names
///   `package` and `per_unit` as needing their own.
/// - [`Variant::LevelAggregation`] on a non-`sum` row — `inst-la-fixture`:
///   "publish of any non-`sum` row without the green joint fixture is blocked
///   (`FIXTURE_MISSING`), exactly like a `modelKind` variant". Read off
///   [`PriceRow::is_level`], which resolves the `sum` default, so an unauthored
///   `aggregationFunction` and an authored `sum` are one row here as everywhere.
/// - [`Variant::SupersessionContinuity`] on a **tiered usage** row — D-22, which
///   ratified the continuity fixture as gating "the first publish of any tiered
///   usage kind (alongside that kind's own fixture)". Both halves are required:
///   `is_tiered` because the fixture is about a tier counter, and `is_usage`
///   because a non-usage row has no counter to continue.
/// - [`Variant::Reserved`] when the caller states the row is reserved — see
///   [`Reservation`] for why that is a parameter.
///
/// It is deliberately a free function and not a method on [`FixtureGate`]: what
/// a row needs is a property of the **row**, decided by the design set, and it
/// does not vary with which registry happens to be loaded. Keeping it separable
/// is what lets a test assert the mapping without a registry at all.
#[must_use]
pub fn required_variants(row: &PriceRow, reservation: Reservation) -> Vec<Variant> {
    let mut required = vec![Variant::ModelKind];
    if row.is_level() {
        required.push(Variant::LevelAggregation);
    }
    if row.is_tiered() && row.is_usage() {
        required.push(Variant::SupersessionContinuity);
    }
    if matches!(reservation, Reservation::Reserved) {
        required.push(Variant::Reserved);
    }
    required
}

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
    /// with `FIXTURE_MISSING` naming the kind and the variant. Returning a
    /// `Result` here would invite a caller to treat the error as "gate unknown"
    /// and proceed, which is the one outcome this type exists to make
    /// unreachable.
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

    /// A gate that is closed for every row.
    ///
    /// The empty registry is not a placeholder for "unknown": an unregistered
    /// `(kind, variant)` pair is never open, so an empty variant list refuses
    /// everything by the same rule that refuses a pair whose fixtures are red.
    #[must_use]
    pub fn closed() -> Self {
        Self {
            registry: Registry::default(),
        }
    }

    /// The kinds whose **own** (`model_kind`) fixture is green, in the corpus
    /// spelling.
    ///
    /// For the boot log, not for the publish path: an operator has to be able
    /// to read from the startup line which kinds are publishable, because the
    /// difference between "the corpus is green" and "the registry file was not
    /// deployed" is otherwise only visible one failed publish at a time.
    ///
    /// It is the `model_kind` variant alone, so it is a **floor** and not the
    /// whole answer — a kind listed here still cannot publish a level or a
    /// tiered usage row unless the matching cross-cutting variant is green too.
    /// [`FixtureGate::open_variants`] is what shows that, and it is what the
    /// boot line logs beside this.
    #[must_use]
    pub fn open_kinds(&self) -> Vec<&'static str> {
        ModelKind::ALL
            .into_iter()
            .filter(|kind| self.registry.gate_open_for(*kind, Variant::ModelKind))
            .map(model_kind_wire)
            .collect()
    }

    /// Every green `(kind, variant)` pair, rendered `kind/variant`.
    ///
    /// The honest boot line. `open_kinds` alone would tell an operator that
    /// `volume` is open while `volume/level_aggregation` is not registered at
    /// all, and the first refusal of a `peak` `volume` row would then read as a
    /// bug rather than as the state of the corpus.
    #[must_use]
    pub fn open_variants(&self) -> Vec<String> {
        let mut open = Vec::new();
        for kind in ModelKind::ALL {
            for variant in Variant::ALL {
                if self.registry.gate_open_for(kind, variant) {
                    open.push(format!("{}/{}", model_kind_wire(kind), variant.wire()));
                }
            }
        }
        open
    }

    /// Whether `row` may be published.
    ///
    /// Asks the registry once per variant in [`required_variants`] and refuses
    /// on the first one that is not green, **naming it** — an operator's next
    /// move is to look that variant up in the registry, and "this row is not
    /// gated" without saying by what is not remediable.
    ///
    /// # Errors
    /// [`DomainError::FixtureMissing`] when a variant the row requires has no
    /// green joint fixture — the corpus has no row for the pair, its oracle half
    /// is unearned, or this gear's validator has not yet reproduced its publish
    /// cases. All three are the same refusal from the caller's side: nobody has
    /// agreed how this shape is evaluated, so it must not become
    /// consumer-visible.
    ///
    /// Also [`DomainError::FixtureMissing`] when the row states no `modelKind`.
    /// The kind is the registry's first key, so there is no fixture to look up
    /// at all; `inst-mk-explicit` has already refused such a row under
    /// `MODEL_KIND_MISSING`, and this gate must not be the one place that reads
    /// an absent kind as an admissible one.
    pub fn check(&self, row: &PriceRow, reservation: Reservation) -> Result<(), DomainError> {
        let Some(kind) = row.model_kind else {
            return Err(DomainError::FixtureMissing(
                "the row states no modelKind, so no conformance fixture resolves for it; \
                 a kind is authored explicitly and is never inferred at publish"
                    .to_owned(),
            ));
        };

        for variant in required_variants(row, reservation) {
            if !self.registry.gate_open_for(kind, variant) {
                return Err(DomainError::FixtureMissing(format!(
                    "model kind `{}` is not gated by a green joint conformance fixture for the \
                     `{}` variant",
                    model_kind_wire(kind),
                    variant.wire()
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "fixture_gate_tests.rs"]
mod fixture_gate_tests;
