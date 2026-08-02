//! The plan aggregate as the domain reasons about it: **one revision of it**,
//! and the fields an open draft may still move.
//!
//! A plan is not a row. It is a chain of revisions keyed `(plan_id, revision)`
//! (`design/01-foundation.md` §3.7 and §4.3, D-56) of which at most one is the
//! **current** one — `published` *or* `retired`, widened by D-128 — and at most
//! one is an open `draft`. The type below is named for the revision rather than
//! for the plan on purpose: a `Plan` type would invite exactly the read D-56
//! removed, "the plan's billing cycle", as though one answer existed
//! independently of which revision is being asked about. Every value here
//! answers *for its revision*.
//!
//! Content freezes at `published` (§4.3): a shape change opens a **new**
//! revision row in `draft`, which publishes through the standard §4.2 path and
//! flips its predecessor `superseded` in the same commit (D-90). That is why
//! [`PlanShapePatch`] is the mutable surface of an *open draft* and not of a
//! plan — there is no edit of a published revision for it to describe.
//!
//! The child shape tables — phases, add-on rules, descriptor set — version with
//! the revision and are copied on a new one (D-83); they are Slice-2 storage and
//! are not modelled here yet.

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::scope_key::PlanId;

/// One revision of a plan: the unit `pricing_plan` stores, the unit the
/// projector sources a plan subject from, and the unit an `ETag` denotes.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevision {
    /// The plan this revision belongs to.
    ///
    /// Stable across the whole chain: the plan's identity, the canonical
    /// scope-key axis and the `pricing_price` attachment all stay on `plan_id`
    /// when a new revision opens (§4.3), so a reprice never has to chase a
    /// moved parent.
    pub plan_id: PlanId,
    /// The revision number, incrementing monotonically from `0` within the
    /// plan.
    ///
    /// Together with [`PlanRevision::plan_id`] it is this row's identity. It is
    /// **not** how "the current revision" is decided — that is the partial
    /// `UNIQUE` index over `published`/`retired` (D-128), storage-defined
    /// precisely so it is never a max-scan convention that two readers could
    /// implement differently.
    pub revision: u64,
    /// The catalog SKU this plan realizes, when one is bound.
    pub sku_id: Option<Uuid>,
    /// The plan's tier.
    ///
    /// A `String` on purpose while this is G3 storage: the enumeration and the
    /// rules that give a tier meaning are Slice-2 semantics. An enum minted
    /// here would fix the value set before the rules that constrain it exist,
    /// and the first tier the rules disagreed with would be a migration rather
    /// than a fix.
    pub plan_tier: Option<String>,
    /// The plan's billing cycle, `String` for the same reason as
    /// [`PlanRevision::plan_tier`].
    pub billing_cycle: Option<String>,
    /// Start of the plan's availability window, UTC.
    pub available_from: Option<DateTime<Utc>>,
    /// End of the plan's availability window, UTC.
    pub available_to: Option<DateTime<Utc>>,
    /// Where this revision stands.
    ///
    /// `draft` is the only state whose content may still change
    /// ([`LifecycleState::is_content_mutable`]), and `published`/`retired` are
    /// the two the plan may be *current* in
    /// ([`LifecycleState::is_current_revision`]). Both questions are answered by
    /// the state machine rather than re-spelled at each call site.
    pub lifecycle_state: LifecycleState,
    /// Pseudonymous principal id of the authoring actor.
    ///
    /// Carried on the revision itself so the Slice-12 history surface can read
    /// actor identity under `plan x read`, without the Auditor-only
    /// `pricing_audit_log`.
    pub created_by: Uuid,
    /// When this revision row was created, UTC.
    pub created_at_utc: DateTime<Utc>,
    /// The optimistic-concurrency version this revision is at.
    ///
    /// It moves only while the revision is a draft: a published revision's
    /// content is frozen, and a tag that moved under frozen content would tell
    /// a caller its cached copy is stale when it is not.
    pub row_version: RowVersion,
}

/// The fields an **open draft** revision may still change.
///
/// Every field carries one meaning: `Some(v)` sets the column to `v`, and
/// `None` means **leave it alone**. An entirely absent patch is still a valid
/// request — it asserts the caller's `ETag` and advances it, which is how a
/// no-op edit stays distinguishable from a lost one.
///
/// It is deliberately **not** `Option<Option<T>>`. The double option is the
/// usual way to make "set this nullable column back to NULL" expressible, and
/// G3 has no surface that can express it: the request shape that distinguishes
/// an omitted JSON member from an explicit `null` arrives with the REST layer
/// (G7). Building the double option now would add a state every repository
/// method and every test has to reason about on behalf of a caller that cannot
/// produce it. So this is a **known limitation, stated rather than designed
/// around**: until G7, a `sku_id`, `plan_tier`, `billing_cycle`,
/// `available_from` or `available_to` that has been set cannot be cleared
/// through a patch — only replaced, or abandoned by deleting the draft.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanShapePatch {
    /// Bind the plan to a different catalog SKU.
    pub sku_id: Option<Uuid>,
    /// Move the plan's tier.
    pub plan_tier: Option<String>,
    /// Move the plan's billing cycle.
    pub billing_cycle: Option<String>,
    /// Move the start of the availability window, UTC.
    pub available_from: Option<DateTime<Utc>>,
    /// Move the end of the availability window, UTC.
    pub available_to: Option<DateTime<Utc>>,
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod plan_tests;
