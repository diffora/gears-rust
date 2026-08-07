//! A price row as the store holds it: the authored shape, the canonical scope
//! key it is filed under, and the metadata that is neither.
//!
//! [`PriceRow`] is the **shape** the thirteen Slice-3 rules judge, and it
//! deliberately carries no identity and no key — a rule able to see which row it
//! was looking at would be free to reach a different verdict for two rows that
//! are the same shape. [`ScopeKey`] carries the eight axes. Neither is enough on
//! its own for a caller that has to name a row, tag it, or decide whether it may
//! still be edited, so this module composes them with exactly the columns that
//! answer those three questions.
//!
//! `billing_timing` stays a `String`. `02-plan-definition.md`
//! `inst-cs-recurring` says it in as many words — the requirement is **Slice
//! 6's registered rule**, cross-referenced elsewhere and never re-registered —
//! so an enum minted here would be a second registration of a rule this slice
//! does not own, free to disagree with the one that does.

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::concurrency::RowVersion;
use crate::domain::contracts::ProrationContract;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::ScopeKey;

/// Everything about a price row that an open draft may still change.
///
/// It is deliberately **not** a patch. [`crate::domain::plan::PlanShapePatch`]
/// can be one because its five columns are independent of each other; a price
/// row's are not. Moving `model_kind` from `graduated` to `flat` has to drop
/// the band set and set `amount_minor` in the *same* write, because every
/// intermediate state is one no Slice-3 rule can pass — and a per-field
/// `Some`/`None` encoding cannot say "clear this" at all without the double
/// option `PlanShapePatch` records as a G3 non-goal. So a draft edit submits
/// the content it wants the row to have and the store replaces it wholesale.
///
/// That is also the only encoding under which the band set stays a **set**:
/// bands have no identity a caller could address one at a time, and a partial
/// band update has no geometry for the rules to evaluate.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceContent {
    /// The authored Slice-3 shape, band set included.
    pub row: PriceRow,
    /// Whether the authored amounts are tax-inclusive.
    pub tax_inclusive: bool,
    /// The row's tax category (D-110): the **source of truth**, and the only
    /// place a category lives. `None` states none, which D-154 resolves against
    /// the region taxonomy's default at publish.
    pub tax_category_ref: Option<String>,
    /// `advance` | `arrears`, Slice-6-owned; see the module doc.
    pub billing_timing: Option<String>,
    /// The three proration inputs Subscriptions computes from, when the row has
    /// authored them (`inst-pi-required`).
    ///
    /// Unlike `billing_timing` this **is** typed here, and the difference is not
    /// an inconsistency: the timing's rule is registered by Slice 6 and merely
    /// *named* by Slice 2, so a second enum would be a second registration —
    /// while the proration vocabulary has exactly one owner (K1 says the enum is
    /// owned by Slice 6 and adopted verbatim downstream) and nothing else in the
    /// crate can spell it. See [`crate::domain::contracts`].
    pub proration_contract: Option<ProrationContract>,
    /// The named rounding policy this row resolves against, when one is set on
    /// the row rather than inherited from the tenant default.
    pub rounding_policy_ref: Option<String>,
    /// The grandfathering horizon. Only an `existing_grandfathered` row may
    /// carry one, and once published it may be tightened but never loosened.
    pub grandfather_until: Option<DateTime<Utc>>,
    /// The predecessor this row supersedes on its canonical scope key.
    ///
    /// Set by the two sanctioned producers of `published -> superseded` — the
    /// D-88 supersession unit and the D-100 cutover commit — and it is what
    /// gives the D-127 unit guard its comparison referent. Neither of those
    /// paths exists yet, so in G3 this column is carried and returned and
    /// nothing reads it.
    pub supersedes_price_id: Option<Uuid>,
}

/// One price row, whole: what it is, where it is filed, and what state it is
/// in.
///
/// The fields are flat rather than nesting a [`PriceContent`] because reading a
/// row is the common case and `record.content.row.model_kind` would make every
/// reader pay for a distinction only a writer needs.
/// [`PriceRecord::content`] is the bridge between the two.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceRecord {
    /// The row's identity. Caller-supplied at creation, for the reason
    /// `NewPlanDraft` gives: an authoring surface has to be able to return the
    /// id before the row is durable, and a store that minted ids would make an
    /// idempotent retry create a second row.
    pub price_id: Uuid,
    /// The eight axes this row is the (at most one) current row on.
    pub scope_key: ScopeKey,
    /// The authored shape the Slice-3 rules judge.
    pub row: PriceRow,
    /// Whether the authored amounts are tax-inclusive.
    pub tax_inclusive: bool,
    /// The row's tax category (D-110): the **source of truth**, and the only
    /// place a category lives. `None` states none, which D-154 resolves against
    /// the region taxonomy's default at publish.
    pub tax_category_ref: Option<String>,
    /// `advance` | `arrears`, Slice-6-owned; see the module doc.
    pub billing_timing: Option<String>,
    /// The three proration inputs Subscriptions computes from, when the row has
    /// authored them (`inst-pi-required`).
    ///
    /// Unlike `billing_timing` this **is** typed here, and the difference is not
    /// an inconsistency: the timing's rule is registered by Slice 6 and merely
    /// *named* by Slice 2, so a second enum would be a second registration —
    /// while the proration vocabulary has exactly one owner (K1 says the enum is
    /// owned by Slice 6 and adopted verbatim downstream) and nothing else in the
    /// crate can spell it. See [`crate::domain::contracts`].
    pub proration_contract: Option<ProrationContract>,
    /// The named rounding policy resolved for this row, when one is set on it.
    pub rounding_policy_ref: Option<String>,
    /// The grandfathering horizon, on a grandfathered row.
    pub grandfather_until: Option<DateTime<Utc>>,
    /// The predecessor this row supersedes on its key, when it has one.
    pub supersedes_price_id: Option<Uuid>,
    /// Where the row stands. `draft` is the only state whose content may still
    /// change ([`LifecycleState::is_content_mutable`]).
    pub lifecycle_state: LifecycleState,
    /// Pseudonymous principal id of the authoring actor, carried on the row so
    /// the Slice-12 history surface never needs the Auditor-only audit log.
    pub created_by: Uuid,
    /// When the row was authored, UTC.
    pub created_at_utc: DateTime<Utc>,
    /// The optimistic-concurrency version, and therefore the row's `ETag`.
    ///
    /// It covers the **band set as well as the row**: bands carry no entity tag
    /// of their own, so if a band edit left this alone, two authors editing
    /// different bands of one draft would both satisfy `If-Match` and silently
    /// interleave.
    pub row_version: RowVersion,
}

impl PriceRecord {
    /// The mutable half of this record, ready to be edited and submitted back
    /// under [`PriceRecord::row_version`].
    ///
    /// Read-modify-write is the shape every authoring surface has, and doing it
    /// by hand means restating which columns are content — the restatement that
    /// silently drops one the day a slice adds it.
    #[must_use]
    pub fn content(&self) -> PriceContent {
        PriceContent {
            row: self.row.clone(),
            tax_inclusive: self.tax_inclusive,
            tax_category_ref: self.tax_category_ref.clone(),
            billing_timing: self.billing_timing.clone(),
            proration_contract: self.proration_contract,
            rounding_policy_ref: self.rounding_policy_ref.clone(),
            grandfather_until: self.grandfather_until,
            supersedes_price_id: self.supersedes_price_id,
        }
    }
}

#[cfg(test)]
#[path = "price_record_tests.rs"]
mod price_record_tests;
