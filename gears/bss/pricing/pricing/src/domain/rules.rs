//! The Slice-3 **price-row shape rules** (`design/03-price-structure.md`).
//!
//! Each instruction of the slice becomes one
//! [`ValidationRule`](crate::domain::validation::ValidationRule) over a
//! [`PriceRow`], registered into the Foundation's pipeline. The rules append and
//! never short-circuit: a row with a missing kind *and* a closed top band
//! reports both, because the author remediates a plan in one pass.
//!
//! The `const` codes below are the contract. They are the machine-readable
//! discriminators the design set names verbatim (§5), they are what the
//! conformance corpus's publish cases assert, and they are what an RFC 9457
//! response carries — so they are written once, here, and referenced everywhere
//! rather than spelled at each `report.violate` call site.
//!
//! ## What is deliberately absent
//!
//! `inst-la-units` (the meter must be gauge-kind and the SKU-declared billable
//! unit must equal `level unit x granule`) and `inst-la-composite` (a non-`sum`
//! function on a derived meter) are **not implemented here**. Both are
//! cross-entity checks against the product registry, and what the registry read
//! model carries is not enough for either: since D-372 this module *does* judge a
//! row against the registry ([`price_row_rules`]), but
//! [`CatalogSku`](crate::domain::ports::CatalogSku) declares a metering unit as a
//! **string** and carries neither its kind nor a composite's formula. They are
//! left out rather than stubbed: a rule that
//! always passes is indistinguishable from a rule that holds, and it would make
//! `LEVEL_UNIT_MISMATCH` and `LEVEL_COMPOSITE_FORBIDDEN` read as enforced when
//! nothing enforces them.

pub mod allowance;
pub mod floor_typing;
pub mod level_aggregation;
pub mod model_kind;
pub mod package;
pub mod reservation;
pub mod supersession;
pub mod tier_bands;

use crate::domain::price_row::PriceRow;
use crate::domain::row_sku_rules::{self, RowSkuContext};
use crate::domain::validation::ValidationPipeline;

pub use supersession::SupersessionPair;

// ---------------------------------------------------------------------------
// Blocking codes (design/03-price-structure.md 5)
// ---------------------------------------------------------------------------

/// No explicit `modelKind`. "Tiered (unspecified)" is not publishable.
pub const MODEL_KIND_MISSING: &str = "MODEL_KIND_MISSING";

/// The kind is not legal on this `chargeKind` (D-18).
pub const MODEL_KIND_CHARGEKIND_MISMATCH: &str = "MODEL_KIND_CHARGEKIND_MISMATCH";

/// A non-usage `per_unit` row has no `quantitySource`, or a `manual` source has
/// no quantity.
pub const QUANTITY_SOURCE_MISSING: &str = "QUANTITY_SOURCE_MISSING";

/// `amount_minor` is absent where the kind carries its money, or present where
/// the money lives in the band / package column.
pub const AMOUNT_PLACEMENT_INVALID: &str = "AMOUNT_PLACEMENT_INVALID";

/// The package block fields are missing, out of range, or on a row whose kind
/// does not price in blocks — including a `package` row that also carries bands.
pub const PACKAGE_FIELDS_INVALID: &str = "PACKAGE_FIELDS_INVALID";

/// An evaluation-policy or quantity field on a row whose kind and charge
/// component do not admit it.
pub const EVAL_POLICY_MISPLACED: &str = "EVAL_POLICY_MISPLACED";

/// A usage row without `billingGranularity`, or a tiered / `package` usage row
/// without `tierAggregationWindow`.
pub const EVAL_POLICY_MISSING: &str = "EVAL_POLICY_MISSING";

/// The two windows are each legal and cannot be combined: an hourly tier counter
/// under a billable unit that spans longer than an hour (D-313).
///
/// A separate code from [`EVAL_POLICY_MISPLACED`] because the fault is neither a
/// missing field nor a field on the wrong kind — both values are authorable and
/// both are correct alone. Named after the reason, as this gear's codes are, and
/// deliberately the same shape as `TIER_QUAL_WINDOW_INCOMPATIBLE`, which D-60
/// minted for the other window pair that cannot meet.
pub const TIER_AGG_WINDOW_INCOMPATIBLE: &str = "TIER_AGG_WINDOW_INCOMPATIBLE";

/// Two bands cover the same quantity — including a band set that is not
/// ascending.
pub const TIER_BANDS_OVERLAP: &str = "TIER_BANDS_OVERLAP";

/// The band set leaves a quantity unpriced: a hole between two bands, or a
/// first band that does not start at the origin.
pub const TIER_BANDS_GAP: &str = "TIER_BANDS_GAP";

/// A band covers no quantity (`toQty <= fromQty`).
pub const TIER_BAND_EMPTY: &str = "TIER_BAND_EMPTY";

/// The top band is closed (D-17). Any quantity must stay rateable.
pub const TIER_TOP_CLOSED: &str = "TIER_TOP_CLOSED";

/// A level-aggregation field is on a row that admits none, is missing where one
/// is required, or is out of range (D-44).
pub const LEVEL_FIELDS_INVALID: &str = "LEVEL_FIELDS_INVALID";

/// On a non-`sum` row `billingGranularity` is not the counterpart of
/// `aggregationGranularity` (D-77).
pub const LEVEL_GRANULARITY_MISMATCH: &str = "LEVEL_GRANULARITY_MISMATCH";

/// A successor on an occupied published scope key changes a field the continued
/// `Q` is denominated in, derived from, or priced by (D-82 / D-98 / D-122 /
/// D-127 / D-129).
pub const SUPERSESSION_UNIT_MISMATCH: &str = "SUPERSESSION_UNIT_MISMATCH";

/// A composite meter naming fewer than two constituents (S10 §5,
/// `inst-cm-constituents`). Its sibling `COMPOSITE_CONSTITUENT_UNPUBLISHED` is
/// declared in the design set and **raised nowhere**, because this gear has no
/// registry to ask whether a unit is published — see `plan_rules::composite`.
pub const COMPOSITE_TOO_FEW_CONSTITUENTS: &str = "COMPOSITE_TOO_FEW_CONSTITUENTS";

/// A composite meter that resolves to itself through its constituents, directly
/// or transitively (S10 §5, `inst-cm-formula`).
pub const COMPOSITE_SELF_REFERENCE: &str = "COMPOSITE_SELF_REFERENCE";

/// A band prices a unit **above** the band below it — the non-volume-discount
/// pattern.
///
/// Advisory, never blocking: an increasing ladder is unusual but legitimate
/// (congestion pricing, penalty tiers), so the author is told and the publish
/// proceeds. The design set states the finding (`inst-tb-order`) without naming
/// a code; this is the discriminator the gear reports it under, in the shape the
/// rest of the code set uses.
pub const TIER_BAND_PRICE_INCREASE: &str = "TIER_BAND_PRICE_INCREASE";

// ---------------------------------------------------------------------------
// The row-SKU codes (D-372 I3-I6)
// ---------------------------------------------------------------------------

/// D-372 I3 — a `usage` row on a SKU that declares no metering unit.
pub const USAGE_ROW_SKU_UNMETERED: &str = "USAGE_ROW_SKU_UNMETERED";

/// D-372 I3 — a recurring / one-time row on a SKU that declares a metering unit.
pub const FEE_ROW_SKU_METERED: &str = "FEE_ROW_SKU_METERED";

/// D-372 I4 — the stored `meter` no longer equals the SKU's declaration.
pub const METER_SKU_MISMATCH: &str = "METER_SKU_MISMATCH";

/// D-372 I5 — a `sellable = true` SKU that is not the plan's own.
pub const ROW_SKU_SELLABLE: &str = "ROW_SKU_SELLABLE";

/// D-372 I6 — the row's SKU is not in the registry read model, or is in it under
/// a `status` other than `published`.
///
/// The design set has named this code since Slice 2, and until D-372 **nothing
/// raised it**: the gear had no registry client, and the four modules that name
/// it ([`crate::domain::plan_rules`] and its `composition` / `composite`
/// submodules, and [`crate::domain::contracts`]) each record that as measured
/// absence rather than oversight. It is declared here, beside the four codes
/// D-372 mints, because every code this gear reports is declared in this module.
///
/// What is raised is the **row's** half — `inst-pr-sku-published`, over the SKU a
/// price row names. The plan-level halves those four modules describe (the
/// *parent* SKU's publication state, at adoption and at retirement) are still
/// unraised and still owed.
pub const SKU_NOT_PUBLISHED: &str = "SKU_NOT_PUBLISHED";

/// D-370's reading of a registry SKU: it may not be **newly** named.
///
/// Task 7 serves a deprecated SKU as `status: "published"` plus `deprecated:
/// true`, so [`SKU_NOT_PUBLISHED`] does not fire on it. This code is the
/// introduction refusal; an already-published row that names a since-deprecated
/// SKU is admitted.
pub const ROW_SKU_DEPRECATED: &str = "ROW_SKU_DEPRECATED";

/// Every Slice-3 row-local rule, in report order.
///
/// Ordered by theme — kind, then bands, then package, then level aggregation —
/// so a report reads top-down the way the row is authored. All of these are
/// **row-local** (D-21), which is why they are one pipeline: they run at save
/// *and* again inside the publish commit, and neither run may depend on the
/// other.
///
/// The supersession guard is **not** here. It judges a pair, not a row, so it
/// has its own subject type and its own pipeline
/// ([`supersession_rules`]).
///
/// The **row-SKU** rules are not here either, for the opposite reason: they judge
/// one row, but against the product / SKU registry rather than against the row
/// alone, so they cannot be built without a read. [`price_row_rules`] is this
/// roster with those registry rules in front of it.
#[must_use]
pub fn row_local_rules() -> ValidationPipeline<PriceRow> {
    register_row_local(ValidationPipeline::new())
}

/// The row-local roster, appended to `pipeline`.
///
/// Private and shared by the two public pipelines so the roster is **spelled
/// once**: a second copy is a rule that can leave one pipeline and stay in the
/// other, which is the fault `rules_tests`'s roster assertions exist to catch and
/// the one they could not see.
fn register_row_local(pipeline: ValidationPipeline<PriceRow>) -> ValidationPipeline<PriceRow> {
    pipeline
        .with_rule(Box::new(crate::domain::line_template::LineTemplateValid))
        .with_rule(Box::new(model_kind::ExplicitModelKind))
        .with_rule(Box::new(model_kind::KindRequiredFields))
        .with_rule(Box::new(model_kind::KindForbiddenFields))
        .with_rule(Box::new(model_kind::KindChargeKindMatrix))
        .with_rule(Box::new(tier_bands::BandGeometry))
        .with_rule(Box::new(tier_bands::BandOrigin))
        .with_rule(Box::new(tier_bands::BandTopOpen))
        .with_rule(Box::new(tier_bands::UsageEvaluationPolicy))
        .with_rule(Box::new(package::PackageFields))
        .with_rule(Box::new(package::PackageWindow))
        .with_rule(Box::new(level_aggregation::LevelFields))
        .with_rule(Box::new(level_aggregation::LevelGranularityPairing))
        .with_rule(Box::new(level_aggregation::LevelMaxHold))
        // Slice 10's reservation set (`inst-rv-attrs` / `inst-rv-usage` /
        // `inst-rv-level`). Row-local by construction -- see that module's doc
        // for why they are here rather than in the Foundation plan set, and for
        // what the corpus could not reach while they were not.
        .with_rule(Box::new(reservation::ReservationWellFormed))
        // Slice 10's allowance gate and the compiled set it admits
        // (`inst-ac-gate` / `inst-ac-band`, D-45). Row-local for the reservation
        // set's reason, and registered **after** it so that
        // `ALLOWANCE_WITH_RESERVATION` reads back below the reservation's own
        // findings -- an author who authored both a broken reservation and an
        // allowance beside it fixes the reservation first.
        .with_rule(Box::new(allowance::AllowanceAuthorable))
        .with_rule(Box::new(allowance::CompiledAllowanceWellFormed))
        // Slice 10's floor typing (`inst-ft-fallback` / `inst-ft-warn`).
        // Row-local for the reservation set's reason; see that module for why
        // `FLOOR_TYPE_MISSING` has no rule and where it is owed.
        .with_rule(Box::new(floor_typing::FloorFallbackDeclared))
        .with_rule(Box::new(floor_typing::FloorOutsideBands))
}

/// Every row rule, in report order: the **row-SKU** rules (D-372 I3-I6 and
/// D-370's introduction refusal) and then the row-local roster
/// [`row_local_rules`] registers.
///
/// The registry rules run **first**, and that is the contract rather than a
/// preference. A row naming a SKU this gear cannot read, or one the registry has
/// not published, is answered by that fact before it is answered by anything its
/// own columns say — an author sent to fix a band geometry on a row bound to a
/// SKU that does not exist would fix the wrong thing twice. A deprecated SKU
/// that *is* published is answered next, as [`ROW_SKU_DEPRECATED`], and only
/// when the door says this evaluation introduces the reference.
///
/// `ctx` is one registry read, made at the door and shared by the registry
/// rules (see [`crate::domain::registry_view`]). Building this pipeline costs
/// five `Arc` clones and no I/O.
///
/// Authoring and publish callers supply a fresh request-scoped registry snapshot.
#[must_use]
pub fn price_row_rules(ctx: RowSkuContext) -> ValidationPipeline<PriceRow> {
    register_row_local(registry_row_rules(ctx))
}

/// Registry-only pass for a batch already judged by the local import classifier.
#[must_use]
pub fn registry_row_rules(ctx: RowSkuContext) -> ValidationPipeline<PriceRow> {
    ValidationPipeline::new()
        .with_rule(Box::new(row_sku_rules::RowSkuPublished(ctx.clone())))
        .with_rule(Box::new(row_sku_rules::RowSkuDeprecated(ctx.clone())))
        .with_rule(Box::new(row_sku_rules::RowSkuSellability(ctx.clone())))
        .with_rule(Box::new(row_sku_rules::UsageRowSkuMetered(ctx.clone())))
        .with_rule(Box::new(row_sku_rules::MeterMatchesSku(ctx)))
}

/// The supersession unit guard, as a pipeline over a predecessor/successor pair.
///
/// One rule, deliberately: the guard binds the **key**, not the mechanism, so
/// there is exactly one of it and both sanctioned producers of
/// `published -> superseded` run the same pipeline over the same subject
/// (D-127).
#[must_use]
pub fn supersession_rules() -> ValidationPipeline<SupersessionPair> {
    ValidationPipeline::new().with_rule(Box::new(supersession::SupersessionUnitGuard))
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
