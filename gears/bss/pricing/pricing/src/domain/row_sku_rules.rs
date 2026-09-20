//! Five rules over one [`PriceRow`], all reading the same
//! [`SkuIndex`](crate::domain::registry_view::SkuIndex) — the registry listing
//! the door read once and handed in ([`RowSkuContext`]). They are the first rules
//! in this gear that judge a row against the **product / SKU registry**, which
//! until D-372 it had no client for; `plan_rules`, `plan_rules::composition`,
//! `plan_rules::composite` and `contracts` each record a code they could not
//! raise for exactly that reason, and `SKU_NOT_PUBLISHED` is the first of them to
//! stop being one. `ROW_SKU_DEPRECATED` is D-370's reading of a served deprecated
//! SKU: Task 7 maps it as `status: "published"` plus `deprecated: true`, so
//! publication is not the fault.
//!
//! ## Why the registry rules run first, and why only one of them reads absence
//!
//! `RowSkuPublished` is registered ahead of the others and is the **only**
//! one that treats an absent SKU as a fault. The others return without
//! judging, because every one of their faults is a statement *about the SKU's
//! declaration* — its metering unit, its sellability, whether it is deprecated —
//! and a declaration nobody can read supports no such statement. A row naming a
//! SKU that is not in the listing would otherwise report several consequences of
//! the one fault the author has to fix first, which is the reading
//! `rules::model_kind` fixes the same way for an unauthored `modelKind`.
//!
//! A **deprecated** SKU is the other half and behaves differently on purpose: it
//! *is* in the listing, so its declaration is readable and the other rules judge
//! it. `RowSkuDeprecated` refuses only an **introduction** of the reference; an
//! already-published row that names a since-deprecated SKU still admits. A
//! well-formed new name reports `[ROW_SKU_DEPRECATED]` only — not a cascade with
//! `SKU_NOT_PUBLISHED`.
//!
//! ## The instruction ids
//!
//! `inst-pr-sku-published`, `inst-pr-sku-deprecated`, `inst-pr-sku-role`,
//! `inst-pr-sku-metered` and `inst-pr-meter-derived` are declared by
//! `docs/design/03-price-structure.md` §3, and the codes they report by that
//! document's §5.
//!
//! ## Why these refuse at the **save**, not only at the publish
//!
//! Every violation here is stamped
//! [`Stage::Write`](crate::domain::validation::Stage::Write), through
//! `violate_at_write`, so the price write door keeps it — that door applies
//! [`write_stage_only()`](crate::domain::validation::ValidationReport::write_stage_only),
//! which drops everything else — and the publish report carries it too, a
//! write-stage violation being a violation. D-372's invariants are enforced at
//! save **and** publish, and a publish-stage stamp would have made the door's
//! registry read pointless: it would fetch the listing, judge the row against it,
//! and discard every verdict.
//!
//! This **amends** D-312's line, deliberately. That decision says to stamp a fault
//! write-stage only where *every operand is in the request*, and a registry
//! listing never is. What the door does instead is resolve the operand **before**
//! validation — one
//! [`get_skus`](crate::domain::ports::ProductCatalogClientV1) per request, not
//! per row — so the operand is present when the rule runs, which is what that line
//! was protecting. The criterion underneath it holds unchanged: a write-stage
//! stamp must not refuse an author's legitimate **intermediate** state, and none
//! of these faults has one. `sku_id` is `NOT NULL` and a frozen scope-key axis
//! (I1), `charge_kind` is a frozen axis beside it, and `meter` is *derived and
//! never authored* (I4) — so no later call in the same authoring session can
//! legalise any of the four, and the sole resolution is to retract the row just
//! sent. That is the doctrine's own test, met the way
//! [`rules::package`](crate::domain::rules::package)'s arm meets it. The amended
//! line is recorded by this programme's documentation task.
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
    FEE_ROW_SKU_METERED, METER_SKU_MISMATCH, ROW_SKU_DEPRECATED, ROW_SKU_TYPE_INVALID,
    SKU_NOT_PUBLISHED, USAGE_ROW_SKU_UNMETERED,
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
/// [`Arc`] is why cloning it five times is free: the listing is read once per
/// request and shared, never copied per rule and never held across requests.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuContext {
    /// The SKU the plan itself is sold as — the one SKU a row may name that is
    /// allowed to be `sellable`.
    pub plan_sku: SkuId,
    /// The registry listing every rule here judges against.
    pub index: Arc<SkuIndex>,
    /// Whether this evaluation **introduces** the SKU reference.
    ///
    /// A create, a draft whose `sku_id` just changed, or a draft being published
    /// is an introduction. A row already published in an earlier revision is not:
    /// `PriceRow` alone cannot tell those apart, so the door that built this
    /// context does.
    pub introducing: bool,
}

/// I6 — the row's SKU exists and is `published`. Runs first: every other rule
/// here declines to judge a row whose SKU it cannot read.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuPublished(pub RowSkuContext);

/// D-370 — a deprecated registry SKU may not be **newly** named.
///
/// Registered after [`RowSkuPublished`]: Task 7 serves a deprecated SKU as
/// `status: "published"` plus `deprecated: true`, so publication is not the
/// fault. The operand is an introduction of the reference, carried on
/// [`RowSkuContext::introducing`].
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuDeprecated(pub RowSkuContext);

/// I5 — the plan's own SKU, or a SKU whose **role** is `component`.
///
/// The predicate was `sellable = false` until the roles landed, and the two are
/// not the same question. A sale flag says whether a SKU may be sold; a role
/// says where it may be used. Reading the first as the second made an operator's
/// decision to stop selling an offer into a decision to make it available as
/// every other plan's constituent, and made "I want a constituent" spell itself
/// "declare this unsellable".
///
/// The plan's own SKU keeps its exemption by identity, whatever role it carries:
/// what role *that* SKU may carry is
/// [`plan_sku_rules`](crate::domain::plan_sku_rules)' question, one level up, and
/// asking it here would report the same fault twice.
#[domain_model]
#[derive(Clone, Debug)]
pub struct RowSkuRole(pub RowSkuContext);

/// I3 — `usage` if and only if the SKU declares a metering unit.
#[domain_model]
#[derive(Clone, Debug)]
pub struct UsageRowSkuMetered(pub RowSkuContext);

/// I4 — `meter` equals the value the row's SKU **derives**, on every row.
///
/// The derived value is the SKU's `metering_unit` on a usage row and `None` on
/// any other, because a fee row prices a period and meters nothing. D-372 makes
/// the column derived and never authored, so "equal to the derivation" is the
/// whole of the rule: an authored `Some` on a fee row is as much a mismatch as a
/// stale unit on a usage row, and it is the cell the `is_usage()` guard this rule
/// used to open with left unjudged.
///
/// | `charge_kind` | SKU `metering_unit` | derived `meter` | this rule |
/// | --- | --- | --- | --- |
/// | usage | `Some(u)` | `Some(u)` | judges |
/// | usage | `None` | -- | **declines** |
/// | non-usage | `None` | `None` | judges |
/// | non-usage | `Some(u)` | -- | **declines** |
///
/// The two declining cells are what keeps one fault to one code. A row whose
/// charge kind and whose SKU disagree has **one** thing wrong with it and it is
/// the binding, not the meter — that is `inst-pr-sku-metered`'s fault, and
/// reporting `METER_SKU_MISMATCH` beside `USAGE_ROW_SKU_UNMETERED` would send the
/// author to derive a meter from a SKU that declares none.
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
            Some(sku) => report.violate_at_write(
                SKU_NOT_PUBLISHED,
                "sku_id",
                format!("SKU {} is {}", subject.sku_id, sku.status),
            ),
            None => report.violate_at_write(
                SKU_NOT_PUBLISHED,
                "sku_id",
                format!("SKU {} is not in the registry read model", subject.sku_id),
            ),
        }
    }
}

impl ValidationRule<PriceRow> for RowSkuDeprecated {
    fn name(&self) -> &'static str {
        "inst-pr-sku-deprecated"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        if !self.0.introducing {
            return;
        }
        let Some(sku) = self.0.index.get(subject.sku_id) else {
            return;
        };
        if sku.deprecated {
            report.violate_at_write(
                ROW_SKU_DEPRECATED,
                "sku_id",
                format!("SKU {} is deprecated", subject.sku_id),
            );
        }
    }
}

impl ValidationRule<PriceRow> for RowSkuRole {
    fn name(&self) -> &'static str {
        "inst-pr-sku-role"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(sku) = self.0.index.get(subject.sku_id) else {
            return;
        };
        if subject.sku_id != self.0.plan_sku
            && sku.sku_type != crate::domain::plan_sku_rules::ROLE_COMPONENT
        {
            report.violate_at_write(
                ROW_SKU_TYPE_INVALID,
                "sku_id",
                format!(
                    "SKU {} is a {}; a row must reference its plan's own SKU or a component SKU",
                    subject.sku_id, sku.sku_type
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
            (true, None) => report.violate_at_write(
                USAGE_ROW_SKU_UNMETERED,
                "sku_id",
                format!(
                    "usage row on SKU {} which declares no metering unit",
                    subject.sku_id
                ),
            ),
            (false, Some(unit)) => report.violate_at_write(
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
        let usage = subject.charge_kind.is_usage();
        if usage != sku.metering_unit.is_some() {
            // The binding itself is wrong, and `inst-pr-sku-metered` owns that.
            // Deriving a meter from it would report one fault under two codes.
            return;
        }
        let derived = if usage {
            sku.metering_unit.as_deref()
        } else {
            None
        };
        if subject.meter.as_deref() != derived {
            report.violate_at_write(
                METER_SKU_MISMATCH,
                "meter",
                format!(
                    "stored meter {:?} but a {} row on SKU {} derives {:?}",
                    subject.meter, subject.charge_kind, subject.sku_id, derived
                ),
            );
        }
    }
}

#[cfg(test)]
#[path = "row_sku_rules_tests.rs"]
mod row_sku_rules_tests;
