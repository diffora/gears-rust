//! D-115's **delta domain** — which operand "the row's delta" is taken over, per
//! `modelKind`, and which changes have no computable delta at all.
//!
//! `design/05-governance.md` §3 step 3 states it normatively:
//!
//! > `flat`/`per_unit` → the `amount_minor` delta; `graduated`/`volume` → the
//! > band-wise `unit_price_minor` vector, compared per band **iff the band
//! > geometry (bounds and count) is unchanged**; `package` → the
//! > `package_price_minor` delta **iff `package_size` is unchanged**.
//!
//! Before D-115 "the row's delta" had **no defined operand** for exactly the rows
//! that carry tiered revenue: `amount_minor` is NULL by construction on
//! `graduated`, `volume` and `package` (`AMOUNT_PLACEMENT_INVALID`). This module
//! is that definition, and [`row_delta`] is total over every pair of rows.
//!
//! # `NotComputable` carries **which** field, and that is not decoration
//!
//! The same argument [`MaterialityReason`](super::MaterialityReason) is an enum
//! rather than a bool. Three readers need the field: `pricing_approval.materiality`
//! is a stored column an auditor reads years later and "no delta was computable"
//! does not say whether a band moved or a quantity did; an operator told only
//! "approval required" cannot tell which change to reconsider; and a suite that
//! asserted incomputability without asserting **why** could not tell an
//! implemented arm from a missing one — every such assertion passes against
//! `fn row_delta(..) -> NotComputable("")`.
//!
//! # A change with no computable delta is material, and it is a *registered
//! trigger* rather than a fourth fail-safe
//!
//! D-115 clause (3) puts the whole no-computable-delta class on
//! `inst-mat-registered`'s list, and clause (2) says the geometry and quantity
//! fields are material "regardless of thresholds". So both arms answer with the
//! trigger verdict rather than with a new [`MaterialityReason`]: the no-charge-
//! computation principle forbids this gear deriving an effective-price delta, and
//! a `manual_quantity` of 10 → 1000 multiplies the charge while moving no amount
//! at all. [`super::evaluate`] is where that mapping is made.
//!
//! # Three of D-115's row contract fields are not columns this crate carries
//!
//! The registered set is `billingTiming`, `prorationBasis`,
//! `billing_anchor_policy`, `credit_on_downgrade`, `tax_inclusive`/
//! `tax_category_ref` and `quantity_source`. Three of those are on
//! [`PriceRecord`] and [`PriceRow`] and are compared here.
//! **`prorationBasis`, `billing_anchor_policy` and `credit_on_downgrade` had no
//! column in this gear until 2026-08-07**, and the paragraph is corrected rather
//! than deleted because its argument is what made the absence legitimate: a
//! comparison over a field no row carries is a rule with no operand, so the three
//! were named rather than written — the same "no token without a writer"
//! discipline D-158 and D-175 apply to the audit vocabulary — and it closed
//! *"the slice that adds each column adds its arm below"*.
//!
//! **Slice 6 is that slice.** `m20260802_000050` gives all three an operand
//! through `PriceRecord::proration_contract`, and [`contract_change`] compares
//! them member by member. Member by member rather than as one contract verdict,
//! because D-115 registers three entries and an author told "the proration
//! contract moved" still has to find which of three fields did it.
//!
//! **`tax_category_ref` was the fourth and no longer is**, which makes it a
//! different case from the three above rather than one more of them. Slice 4
//! added the column (`m20260802_000037`), so the operand exists — and **no arm
//! compares it**, so a supersession changing only a row's tax category is still
//! classified immaterial. That is not the "no column, so no rule" argument this
//! heading makes; it is a rule with an operand and no comparison, and it is owed
//! rather than declined (`T-14`). D-48 makes `taxCategory` one of the five
//! elements Billing countersigns, so a change to it that no second principal sees
//! is the case materiality exists for.

use toolkit_macros::domain_model;

use crate::domain::money::RATE_SUB_DECIMALS;
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow, TierBand};

/// One amount's move, carrying **both** ends.
///
/// The delta alone cannot answer a percent threshold: `percent` is a ratio and a
/// ratio needs its denominator. Carrying the pair keeps the operand and the
/// baseline it is a fraction of in one place, so "which amount is the delta over"
/// and "which amount is the percentage of" cannot be answered by two different
/// pieces of code — which is the divergence D-115 exists to close, one level down.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmountMove {
    /// The baseline amount, in the units [`Self::scale`] names.
    pub from_minor: i64,
    /// The proposed amount, same units.
    pub to_minor: i64,
    /// Which of the two things this move is about (D-311).
    ///
    /// It rides the move rather than being inferred by the comparer because the
    /// comparer is handed a slice and cannot see which row produced it.
    pub scale: MoveScale,
}

/// Whether a move is between **amounts** or between **rates** (D-311).
///
/// The distinction is what keeps a threshold meaning the same thing it meant
/// before rates got their own scale. `pricing_threshold.absolute_minor` is
/// authored in minor units, and a raw `>=` against a nano-minor move would read
/// a bar of 5 as `0.000000005` — every band change auto-publishing, which is the
/// governance control silently inverted. The scale rides the move so the
/// comparer can put the bar into the operand's units before comparing.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoveScale {
    /// Whole ISO-4217 minor units — a `flat` amount or a package price.
    #[default]
    Minor,
    /// 10^-9 minor units — a band rate or a `per_unit` rate.
    NanoMinor,
}

impl MoveScale {
    /// How many stored units make one minor unit, so a bar authored in minor
    /// units can be raised into this scale.
    const fn per_minor_unit(self) -> i128 {
        match self {
            Self::Minor => 1,
            Self::NanoMinor => 10_i128.pow(RATE_SUB_DECIMALS),
        }
    }

    /// The token the stored `materiality` document and the wire carry, so the
    /// two amounts a verdict reports are readable as the units they were
    /// measured in.
    ///
    /// `MaterialityReason::as_str`'s shape and reason: the rendering belongs to
    /// the type that owns the roster, so a variant added later cannot acquire a
    /// spelling at a call site instead of here.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::NanoMinor => "nanoMinor",
        }
    }
}

impl AmountMove {
    /// How far it moved, unsigned.
    ///
    /// Unsigned because a **cut** is as material as a rise: an overlay edit from
    /// −10% to −90% is the D-50 hazard in the other direction, and a threshold
    /// that only looked at increases would wave every price cut through.
    #[must_use]
    pub const fn magnitude_minor(self) -> i64 {
        self.to_minor
            .saturating_sub(self.from_minor)
            .saturating_abs()
    }

    /// Is the move at or above `absolute_minor`?
    ///
    /// `>=` rather than `>`, and the boundary is decided rather than incidental:
    /// `cpt-cf-bss-pricing-dod-threshold` says auto-publish happens "only **below** an explicit threshold",
    /// so a move that reaches the threshold is not below it. It is also what makes
    /// a configured threshold of `0` mean "everything is material" — the reading
    /// the store's `absolute_minor >= 0` CHECK admits zero for.
    ///
    /// **The bar is raised into the move's scale (D-311), not the move lowered
    /// into the bar's.** Rounding a nano-minor move down to whole minor units
    /// would floor every sub-cent rate change to zero, which is the truncation
    /// D-311 exists to end, arriving one layer further in. `i128` throughout so
    /// the 10⁹ multiplication of a large configured bar cannot wrap — a wrapped
    /// bar goes negative and every move "reaches" it.
    ///
    /// A band's bar always compared against the **per-unit** band price, before
    /// this scale existed and after it; D-311 changed how that number is stored,
    /// not what the tenant configured it to mean.
    #[must_use]
    pub const fn reaches_absolute(self, absolute_minor: i64) -> bool {
        let bar = (absolute_minor as i128).saturating_mul(self.scale.per_minor_unit());
        (self.magnitude_minor() as i128) >= bar
    }

    /// Is the move at or above `percent_bp` of its baseline?
    ///
    /// `None` when no percentage is computable — a zero baseline, which step 3 of §3
    /// names explicitly: *"A **percent-only** policy against a zero (or NULL)
    /// baseline is likewise material — no percentage is computable."* The caller
    /// turns that `None` into the material verdict rather than into a division.
    ///
    /// The comparison is `|delta| * 10_000 >= percent_bp * baseline`, cross-
    /// multiplied so it is integer throughout: there is no floating point beside
    /// money here, which is the reason the store holds basis points at all.
    #[must_use]
    pub const fn reaches_percent(self, percent_bp: i64) -> Option<bool> {
        if self.from_minor == 0 {
            return None;
        }
        let scaled = self.magnitude_minor().saturating_mul(10_000);
        let bar = self.from_minor.saturating_abs().saturating_mul(percent_bp);
        Some(scaled >= bar)
    }
}

/// What a row's delta **is**, per D-115.
///
/// Total over every pair of rows: the fourth arm is what makes it total, and it
/// names the field that made the comparison impossible rather than answering an
/// empty vector or a zero.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowDelta {
    /// `flat` / `per_unit` — the `amount_minor` move.
    Amount(AmountMove),
    /// `graduated` / `volume` — the band-wise `unit_price_minor` moves, in band
    /// order, on unchanged geometry. Any band over its threshold trips the row,
    /// which is §3's any-row-trips reading (G3) one level down.
    BandVector(Vec<AmountMove>),
    /// `package` — the `package_price_minor` move, on an unchanged
    /// `package_size`.
    PackagePrice(AmountMove),
    /// No delta is computable, and this is the field that made it so.
    NotComputable(&'static str),
}

/// The band geometry two band sets must share for a band-wise comparison to mean
/// anything: the same count, and the same bounds in the same order.
///
/// A geometry change is **not** a delta of zero and not a delta at all. Moving
/// `[0,1000)` to `[0,10)` at an identical unit price multiplies what a subscriber
/// pays, and a band-wise comparison over two differently-shaped vectors would be
/// comparing a band to a different band.
fn geometry_of(bands: &[TierBand]) -> Vec<(u64, Option<u64>)> {
    bands
        .iter()
        .map(|band| (band.from_qty, band.to_qty.closed_at()))
        .collect()
}

/// The fields that move money at **zero amount delta** — quantity-determining, or
/// money on an axis this domain cannot compute.
///
/// §3 step 3 names them and D-115 clause (2) states the rule: *"no effective-price
/// delta is computable catalog-side (the no-charge-computation principle forbids
/// computing one), so the G1 fail-safe applies"*. `package_size` is here too, and
/// is checked before the `package` arm reads its price, because a package price
/// held constant over a halved package size is a doubling.
fn quantity_change(current: &PriceRow, baseline: &PriceRow) -> Option<&'static str> {
    if current.manual_quantity != baseline.manual_quantity {
        return Some("manual_quantity");
    }
    if current
        .included_allowance
        .map(|allowance| allowance.quantity)
        != baseline
            .included_allowance
            .map(|allowance| allowance.quantity)
    {
        return Some("includedAllowance.quantity");
    }
    // --- Slice 10's primitives (D-254) ---
    //
    // Added a wave after the columns landed, which is the defect this arm
    // records rather than merely fixes: none of them reached this domain, so a
    // successor moving only one of them produced a **zero** amount move and was
    // classified immaterial -- a publish on one principal.
    //
    // `reserved_rate_minor` is money and is nonetheless `NotComputable` for
    // clause (2)'s reason: the effective delta needs the covered-granule count
    // (D-139), which is Rating's runtime fact and not the catalog's to compute.
    if current.reserved_rate_minor != baseline.reserved_rate_minor {
        return Some("reserved_rate_minor");
    }
    // The three the evaluation-policy roster already calls quantity-determining.
    // A field the roster files as deciding the billable quantity and this domain
    // treats as immaterial is the contradiction `includedAllowance.quantity`
    // was added here to close.
    if current.reservation_flavor != baseline.reservation_flavor {
        return Some("reservation_flavor");
    }
    if current.min_qty_usage != baseline.min_qty_usage {
        return Some("min_qty_usage");
    }
    if current.min_qty_usage_fallback != baseline.min_qty_usage_fallback {
        return Some("min_qty_usage_fallback");
    }
    None
}

/// The row **contract** fields this crate carries, from D-115 clause (3)'s set.
///
/// Four of the seven have no column here; the module doc names them and says why
/// an arm for each would be a rule with no operand.
fn contract_change(current: &PriceRecord, baseline: &PriceRecord) -> Option<&'static str> {
    if current.billing_timing != baseline.billing_timing {
        return Some("billingTiming");
    }
    if current.tax_inclusive != baseline.tax_inclusive {
        return Some("tax_inclusive");
    }
    // Beside `tax_inclusive` because D-115 registers the two as one entry, and
    // after it because a row that moved both is described by the basis it is
    // billed on before the category it is billed under. `None` is *the row states
    // none* and not "no category" (D-154 resolves it against the region default
    // at publish), so either direction of the move is a change to what Billing
    // countersigns.
    if current.tax_category_ref != baseline.tax_category_ref {
        return Some("tax_category_ref");
    }
    if current.row.quantity_source != baseline.row.quantity_source {
        return Some("quantity_source");
    }
    // D-115's three remaining row-contract fields, written now that Slice 6's
    // `m20260802_000050` gives them operands. They are reported **member by
    // member** rather than as one `proration_contract` verdict, because D-115
    // registers three entries and an author told "the proration contract moved"
    // still has to find which of three fields did.
    //
    // Absent-vs-present counts as a change in either direction: the whole set is
    // required on a recurring row (`inst-pi-required`), so a row that gained or
    // lost it is a row whose consumer contract moved.
    let anchors = (
        current.proration_contract.map(|c| c.billing_anchor_policy),
        baseline.proration_contract.map(|c| c.billing_anchor_policy),
    );
    if anchors.0 != anchors.1 {
        return Some("billing_anchor_policy");
    }
    let bases = (
        current.proration_contract.map(|c| c.proration_basis),
        baseline.proration_contract.map(|c| c.proration_basis),
    );
    if bases.0 != bases.1 {
        return Some("prorationBasis");
    }
    let credits = (
        current.proration_contract.map(|c| c.credit_on_downgrade),
        baseline.proration_contract.map(|c| c.credit_on_downgrade),
    );
    if credits.0 != credits.1 {
        return Some("credit_on_downgrade");
    }
    None
}

/// The delta between a proposed row and the row it replaces on its scope key.
///
/// The order the checks run in is the order §3 step 3 states them, and it matters
/// in one place: the quantity and contract fields are asked **before** the
/// per-kind operand, because a row that changed both its amount and its
/// `manual_quantity` has no computable delta — reporting the amount move would
/// invite a threshold comparison over the smaller of the two changes.
///
/// A `model_kind` move is its own incomputable arm rather than an amount
/// comparison across kinds: a `flat` row becoming `graduated` has no operand in
/// common with its predecessor at all.
#[must_use]
pub fn row_delta(current: &PriceRecord, baseline: &PriceRecord) -> RowDelta {
    if let Some(field) = contract_change(current, baseline) {
        return RowDelta::NotComputable(field);
    }
    if let Some(field) = quantity_change(&current.row, &baseline.row) {
        return RowDelta::NotComputable(field);
    }
    let (Some(kind), Some(baseline_kind)) = (current.row.model_kind, baseline.row.model_kind)
    else {
        return RowDelta::NotComputable("model_kind");
    };
    if kind != baseline_kind {
        return RowDelta::NotComputable("model_kind");
    }
    match kind {
        ModelKind::Flat | ModelKind::PerUnit => amount_delta(current, baseline),
        ModelKind::Graduated | ModelKind::Volume => band_delta(current, baseline),
        ModelKind::Package => package_delta(current, baseline),
    }
}

fn amount_delta(current: &PriceRecord, baseline: &PriceRecord) -> RowDelta {
    // **`per_unit` moved out of `amount_minor` (D-311)**, so the two kinds this
    // arm used to serve are now two reads. A `per_unit` row's price is a rate,
    // and comparing it in minor units against a minor-unit threshold is the
    // arithmetic that made a third of the price look like nothing.
    if current.row.model_kind == Some(crate::domain::price_row::ModelKind::PerUnit) {
        let (Some(to), Some(from)) = (current.row.unit_rate, baseline.row.unit_rate) else {
            return RowDelta::NotComputable("unit_rate");
        };
        return RowDelta::Amount(AmountMove {
            from_minor: from.nano_minor(),
            to_minor: to.nano_minor(),
            scale: MoveScale::NanoMinor,
        });
    }
    let (Some(to), Some(from)) = (current.row.amount_minor, baseline.row.amount_minor) else {
        return RowDelta::NotComputable("amount_minor");
    };
    RowDelta::Amount(AmountMove {
        from_minor: from.get(),
        to_minor: to.get(),
        scale: MoveScale::Minor,
    })
}

fn band_delta(current: &PriceRecord, baseline: &PriceRecord) -> RowDelta {
    // **This arm refuses nothing the geometry arm below would not**, and that is
    // reported rather than implied: two band sets of different lengths always
    // produce different geometry vectors, so deleting this arm leaves a count
    // change refused as `"tier band bounds"`. What it earns is the **sentence**,
    // which is the reason `NotComputable` carries a field at all — a band added or
    // removed and a band whose bound moved are different acts, and an operator
    // reconsiders a different thing for each. Its guard-by-removal proof is
    // therefore a proof about the reason, not about the refusal.
    if current.row.bands.len() != baseline.row.bands.len() {
        return RowDelta::NotComputable("tier band count");
    }
    if geometry_of(&current.row.bands) != geometry_of(&baseline.row.bands) {
        return RowDelta::NotComputable("tier band bounds");
    }
    RowDelta::BandVector(
        current
            .row
            .bands
            .iter()
            .zip(baseline.row.bands.iter())
            .map(|(to, from)| AmountMove {
                from_minor: from.unit_price_rate.nano_minor(),
                to_minor: to.unit_price_rate.nano_minor(),
                scale: MoveScale::NanoMinor,
            })
            .collect(),
    )
}

fn package_delta(current: &PriceRecord, baseline: &PriceRecord) -> RowDelta {
    if current.row.package_size != baseline.row.package_size {
        return RowDelta::NotComputable("package_size");
    }
    let (Some(to), Some(from)) = (
        current.row.package_price_minor,
        baseline.row.package_price_minor,
    ) else {
        return RowDelta::NotComputable("package_price_minor");
    };
    RowDelta::PackagePrice(AmountMove {
        from_minor: from.get(),
        to_minor: to.get(),
        // A package price is what the block costs — an amount on the invoice, so
        // the currency's own scale is right for it.
        scale: MoveScale::Minor,
    })
}

#[cfg(test)]
#[path = "delta_tests.rs"]
mod delta_tests;
