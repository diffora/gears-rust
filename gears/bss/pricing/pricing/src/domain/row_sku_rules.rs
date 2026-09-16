//! D-372 I3-I6: a price row is bound to the registry SKU it names.
//!
//! Four rules over one [`PriceRow`], all reading the same
//! [`SkuIndex`](crate::domain::registry_view::SkuIndex) — the registry listing
//! the door read once and handed in ([`RowSkuContext`]). They are the first rules
//! in this gear that judge a row against the **product / SKU registry**, which
//! until D-372 it had no client for; `plan_rules`, `plan_rules::composition`,
//! `plan_rules::composite` and `contracts` each record a code they could not
//! raise for exactly that reason, and `SKU_NOT_PUBLISHED` is the first of them to
//! stop being one.
//!
//! ## Why the registry rules run first, and why only one of them reads absence
//!
//! `RowSkuPublished` is registered ahead of the other three and is the **only**
//! one that treats an absent SKU as a fault. The other three return without
//! judging, because every one of their faults is a statement *about the SKU's
//! declaration* — its metering unit, its sellability — and a declaration nobody
//! can read supports no such statement. A row naming a SKU that is not in the
//! listing would otherwise report three consequences of the one fault the author
//! has to fix first, which is the reading `rules::model_kind` fixes the same way
//! for an unauthored `modelKind`.
//!
//! A **deprecated** SKU is the other half and behaves differently on purpose: it
//! *is* in the listing, so its declaration is readable and the other three judge
//! it. The author is told the SKU may no longer be adopted **and** what else is
//! wrong with the row.
//!
//! ## What is deliberately absent
//!
//! Nothing here judges the SKU's `plan_tier`, `sku_type` or `usage_type_ref`.
//! Those are `PLANTIER_DIVERGENT`, the composition set's tier drift, and
//! `METER_USAGE_TYPE_UNBOUND` / `METER_DIMENSION_UNDECLARED` — all named in
//! [`plan_rules`](crate::domain::plan_rules)'s absence table, all judged against
//! a **plan** rather than a row, and none of them D-372's. A rule that read those
//! fields here would answer a plan-level question from one row's point of view.

use std::sync::Arc;

use toolkit_macros::domain_model;

use crate::domain::price_row::PriceRow;
use crate::domain::registry_view::SkuIndex;
use crate::domain::rules::{
    FEE_ROW_SKU_METERED, METER_SKU_MISMATCH, ROW_SKU_SELLABLE, SKU_NOT_PUBLISHED,
    USAGE_ROW_SKU_UNMETERED,
};
use crate::domain::scope_key::SkuId;
use crate::domain::validation::{ValidationReport, ValidationRule};

/// The `status` value a row's SKU must carry, verbatim registry vocabulary.
///
/// Not an enum, for [`CatalogSku::status`](crate::domain::ports::CatalogSku)'s
/// own reason: the registry owns this vocabulary, and a fifth state must not
/// become a parse failure in the gear that merely checks it against one value.
const PUBLISHED: &str = "published";

/// One registry read plus the plan's own SKU, as the rules see them.
///
/// Built at the door and cloned into each rule. The
/// [`Arc`] is why cloning it four times is free: the listing is read once per
/// request and shared, never copied per rule and never held across requests.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuContext {
    /// The SKU the plan itself is sold as — the one SKU a row may name that is
    /// allowed to be `sellable`.
    pub plan_sku: SkuId,
    /// The registry listing every rule here judges against.
    pub index: Arc<SkuIndex>,
}

/// I6 — the row's SKU exists and is `published`. Runs first: every other rule
/// here declines to judge a row whose SKU it cannot read.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuPublished(pub RowSkuContext);

/// I5 — the plan's own SKU, or a `sellable = false` SKU.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuSellability(pub RowSkuContext);

/// I3 — `usage` if and only if the SKU declares a metering unit.
#[domain_model]
#[derive(Clone, Debug)]
pub struct UsageRowSkuMetered(pub RowSkuContext);

/// I4 — a stored `meter` equals the SKU's declaration.
#[domain_model]
#[derive(Clone, Debug)]
pub struct MeterMatchesSku(pub RowSkuContext);

impl ValidationRule<PriceRow> for RowSkuPublished {
    fn name(&self) -> &'static str {
        "inst-pr-sku-published"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        match self.0.index.get(subject.sku_id) {
            Some(sku) if sku.status == PUBLISHED => {}
            Some(sku) => report.violate(
                SKU_NOT_PUBLISHED,
                "sku_id",
                format!("SKU {} is {}", subject.sku_id, sku.status),
            ),
            None => report.violate(
                SKU_NOT_PUBLISHED,
                "sku_id",
                format!("SKU {} is not in the registry read model", subject.sku_id),
            ),
        }
    }
}

impl ValidationRule<PriceRow> for RowSkuSellability {
    fn name(&self) -> &'static str {
        "inst-pr-sku-sellability"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(sku) = self.0.index.get(subject.sku_id) else {
            return;
        };
        if subject.sku_id != self.0.plan_sku && sku.sellable {
            report.violate(
                ROW_SKU_SELLABLE,
                "sku_id",
                format!(
                    "SKU {} is sellable on its own; a plan that sells it is a bundle, not a row",
                    subject.sku_id
                ),
            );
        }
    }
}

impl ValidationRule<PriceRow> for UsageRowSkuMetered {
    fn name(&self) -> &'static str {
        "inst-pr-sku-metered"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(sku) = self.0.index.get(subject.sku_id) else {
            return;
        };
        match (subject.charge_kind.is_usage(), sku.metering_unit.as_deref()) {
            (true, None) => report.violate(
                USAGE_ROW_SKU_UNMETERED,
                "sku_id",
                format!(
                    "usage row on SKU {} which declares no metering unit",
                    subject.sku_id
                ),
            ),
            (false, Some(unit)) => report.violate(
                FEE_ROW_SKU_METERED,
                "sku_id",
                format!(
                    "{} row on SKU {} which declares unit {unit}",
                    subject.charge_kind, subject.sku_id
                ),
            ),
            (true, Some(_)) | (false, None) => {}
        }
    }
}

impl ValidationRule<PriceRow> for MeterMatchesSku {
    fn name(&self) -> &'static str {
        "inst-pr-meter-derived"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(sku) = self.0.index.get(subject.sku_id) else {
            return;
        };
        if subject.charge_kind.is_usage()
            && subject.meter.as_deref() != sku.metering_unit.as_deref()
        {
            report.violate(
                METER_SKU_MISMATCH,
                "meter",
                format!(
                    "stored meter {:?} but SKU {} declares {:?}",
                    subject.meter, subject.sku_id, sku.metering_unit
                ),
            );
        }
    }
}

#[cfg(test)]
#[path = "row_sku_rules_tests.rs"]
mod row_sku_rules_tests;
