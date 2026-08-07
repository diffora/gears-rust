//! Retirement's compose-time judgement (`inst-rt-cancel`, `inst-re-cancelflow`,
//! `inst-re-references`, D-51, D-131, D-182).
//!
//! Retirement stops **selling**, never **rating**. Everything in this module
//! follows from that one sentence: not-sellable comes from the lifecycle
//! predicate of the sellability gate (`domain::sellability` predicate (4)), so
//! nothing here has to cancel a window in order to stop a sale, and every window
//! it *does* cancel it cancels for a different reason — an unoccupied key's
//! scheduled window is coverage nobody is going to use.
//!
//! # The presence question is asked once, and answered per price id
//!
//! D-131. Retirement submits the **union** of its keys' price-id sets in a single
//! call on the D-79 in-flight-subscription lane and reads a **per-price-id
//! presence map** back. The two alternatives it was chosen over are both wrong in
//! ways worth keeping written down: a lane that answers a *count* over the
//! submitted set answers only "does this plan have any subscriber at all", under
//! which retirement cancels nothing whenever a single key is occupied; and a
//! per-key call puts N synchronous cross-gear round trips (N = keys × markets)
//! inside the retirement transaction, holding the price and window row locks and
//! the audit chain segment across the fan-out.
//!
//! # The lane is absent, so every key reads occupied
//!
//! D-182, normative. The D-79 lane has no client, no contract type and no
//! counterpart gear in the built system. D-131's outage clause is fail-closed —
//! on lane outage or timeout, **windows are kept** — and D-182 makes an absent
//! lane that same case. So in the system as built [`PresenceMap::fail_closed`] is
//! the only value this module is ever handed, every key reads occupied, and
//! retirement keeps every scheduled window rather than cancelling on the keys the
//! design set would have let it cancel on.
//!
//! That is not a stub standing in for the rule: it *is* the rule, evaluated
//! against the world as it is. [`dispose_windows`] is written against the map and
//! not against its absence, so the day the lane client lands, a
//! [`PresenceMap::resolved`] flows through the same code and the behaviour
//! becomes what `inst-rt-cancel` describes with no edit here.
//!
//! # Kept is not the same fact as cancelled, and the operator sees both
//!
//! `inst-re-cancelflow` requires the confirm screen to label kept windows
//! distinctly from cancelled ones, which is why [`WindowDisposition`] carries a
//! reason rather than a bool: an operator who is shown "3 windows" cannot tell a
//! plan whose coverage is being preserved from one whose coverage is being torn
//! down.

use std::collections::BTreeSet;

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// Wire code for a retirement refused by a blocking reference
/// (`11-lifecycle.md` §5, `inst-re-references`, 409).
pub const RETIRE_PLAN_REFERENCED: &str = "RETIRE_PLAN_REFERENCED";

/// Which price ids carry in-flight subscribers (D-51's predicate, D-131's shape).
///
/// Built from **one** lane call over the union of the retiring plan's price ids.
/// The absent-lane and outage cases collapse into [`PresenceMap::fail_closed`],
/// because D-131 gives them the same answer and D-182 makes the first of them the
/// only one this system reaches.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceMap {
    /// The price ids the lane reported in-flight subscribers on. Meaningless
    /// when `fail_closed` is set, and empty there so a reader that ignores the
    /// flag still cannot conclude "unoccupied" from a lookup.
    occupied: BTreeSet<Uuid>,
    /// The lane could not answer. Every lookup then reads occupied.
    fail_closed: bool,
}

impl PresenceMap {
    /// The lane answered: exactly `occupied` carry in-flight subscribers.
    #[must_use]
    pub fn resolved(occupied: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            occupied: occupied.into_iter().collect(),
            fail_closed: false,
        }
    }

    /// The lane is absent, out, or timed out (D-131's fail-closed clause; D-182
    /// makes it the built system's only case).
    #[must_use]
    pub fn fail_closed() -> Self {
        Self {
            occupied: BTreeSet::new(),
            fail_closed: true,
        }
    }

    /// Does this price id carry in-flight subscribers?
    ///
    /// `true` for **every** id when the map failed closed — which is the whole
    /// of the fail-closed posture, stated once here rather than at each caller.
    #[must_use]
    pub fn is_occupied(&self, price_id: Uuid) -> bool {
        self.fail_closed || self.occupied.contains(&price_id)
    }

    /// Did the lane answer at all?
    ///
    /// The dry-run preview reports this, because "kept because the lane could not
    /// tell us" and "kept because a subscriber is on it" are different facts for
    /// the operator confirming, and `inst-re-warn` puts the list in front of them.
    #[must_use]
    pub const fn is_fail_closed(&self) -> bool {
        self.fail_closed
    }
}

/// A scheduled window retirement has to decide about.
///
/// Only **not-yet-active** windows are candidates: an active window runs to its
/// natural end for the in-flight subscribers this slice preserves coverage for,
/// so it is never a member of the input set at all.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledWindow {
    /// The window row.
    pub window_id: Uuid,
    /// The price row it schedules — the presence map's key (D-131).
    pub price_id: Uuid,
}

/// Why a scheduled window is being kept.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// The key has in-flight subscribers, so the window is their continuing
    /// coverage (D-51). Cancelling it opens the trailing void no gap check can
    /// see: the active window expires at its natural end (`inst-ws-expire`) and
    /// every arrears charge and renewal after it fails closed.
    InFlightSubscribers,
    /// The lane could not be asked, so the key reads occupied (D-131's
    /// fail-closed clause, D-182's absent lane).
    PresenceUnresolved,
}

/// What retirement will do to one scheduled window.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDisposition {
    /// The key has no in-flight subscribers: Slice 7's cancellation flow is
    /// invoked on it (`inst-re-cancelflow` — invoked, never marked invalid, so
    /// `PriceWindowCancelled` is emitted and the consumer caches evict).
    Cancelled,
    /// The window stands, for this reason.
    Kept(KeptReason),
}

/// One window and what retirement will do to it.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowVerdict {
    /// The window row.
    pub window_id: Uuid,
    /// The price row it schedules.
    pub price_id: Uuid,
    /// Cancelled, or kept and why.
    pub disposition: WindowDisposition,
}

impl WindowVerdict {
    /// Is this window one the cancellation flow will be invoked on?
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self.disposition, WindowDisposition::Cancelled)
    }
}

/// Decide every scheduled window against one presence map (D-51, D-131).
///
/// The map is asked per **price id**, which is what makes a plan with mixed keys
/// — some occupied, some not — cancel exactly the unoccupied ones. A union count
/// could not distinguish them, and that is D-131's whole argument.
#[must_use]
pub fn dispose_windows(
    scheduled: &[ScheduledWindow],
    presence: &PresenceMap,
) -> Vec<WindowVerdict> {
    scheduled
        .iter()
        .map(|w| WindowVerdict {
            window_id: w.window_id,
            price_id: w.price_id,
            disposition: if presence.is_occupied(w.price_id) {
                WindowDisposition::Kept(if presence.is_fail_closed() {
                    KeptReason::PresenceUnresolved
                } else {
                    KeptReason::InFlightSubscribers
                })
            } else {
                WindowDisposition::Cancelled
            },
        })
        .collect()
}

/// A reference to the retiring plan that **blocks** the retirement
/// (`inst-re-references`).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockingReferenceKind {
    /// The plan is a component of a bundle (`sum_of_parts` / `own_price`
    /// composition, Slice 8). Remediation is to re-compose the bundle or retire
    /// it first.
    BundleComponent,
    /// The plan is the target of an add-on price override (Slice 2).
    AddOnPriceOverrideTarget,
}

/// A reference that is **enumerated and does not block**.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningReferenceKind {
    /// Another plan lists the retiree in `allowedChangeTargets` (D-24). The edge
    /// goes inert rather than dangling: Subscriptions re-checks the target's
    /// lifecycle state at change time, so blocking here would refuse a retirement
    /// on the strength of an edge nobody can traverse anyway.
    AllowedChangeTarget,
    /// A `PriceOverlay` targets the retiree (D-31). It goes dangling-and-flagged
    /// (`pricing.priceoverlay.target_retired`) and stays evaluable for in-flight
    /// subscribers, which is the same reason the windows above are kept.
    PriceOverlayTarget,
}

/// One reference to the retiring plan, of either weight.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanReference {
    /// The bundle, plan or overlay that refers to the retiree.
    pub referrer_id: Uuid,
    /// A caller-facing name for the referrer, so the dry-run enumerates something
    /// an operator can act on rather than a bare id.
    pub referrer_label: String,
}

/// What refers to the plan, blocking and otherwise (`inst-re-references`).
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceReport {
    /// Blocking references, by kind.
    pub blocking: Vec<(BlockingReferenceKind, PlanReference)>,
    /// Enumerated-only references, by kind.
    pub warnings: Vec<(WarningReferenceKind, PlanReference)>,
}

impl ReferenceReport {
    /// Refuse the retirement if anything blocking refers to the plan.
    ///
    /// The refusal enumerates the referrers, because "this plan is referenced" is
    /// not an actionable sentence: the remediation is to re-compose or retire a
    /// *named* referrer first.
    ///
    /// # Errors
    /// [`DomainError::RetirePlanReferenced`] listing every blocking referrer.
    pub fn ensure_retirable(&self, plan_id: Uuid) -> Result<(), DomainError> {
        if self.blocking.is_empty() {
            return Ok(());
        }
        let references = self
            .blocking
            .iter()
            .map(|(kind, r)| format!("{} {} ({})", kind.as_str(), r.referrer_label, r.referrer_id))
            .collect::<Vec<_>>()
            .join(", ");
        Err(DomainError::RetirePlanReferenced(format!(
            "plan {plan_id} cannot be retired while it is referenced by: {references}; re-compose \
             or retire the referrer first"
        )))
    }
}

impl BlockingReferenceKind {
    /// The token the refusal and the dry-run render.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BundleComponent => "bundle component",
            Self::AddOnPriceOverrideTarget => "add-on price-override target",
        }
    }
}

impl WarningReferenceKind {
    /// The token the dry-run renders.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllowedChangeTarget => "allowed change target",
            Self::PriceOverlayTarget => "price overlay target",
        }
    }
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod retirement_tests;
