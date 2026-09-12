//! The state-machine floor: which `lifecycle_state` changes the Foundation
//! admits, what a terminal row refuses, and what an admitted transition costs
//! the row (`design/01-foundation.md` §2, `inst-fd-transition-guard` and its
//! sub-instructions, plus `inst-fd-terminal`).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-transition-guard:p1
//!
//! # Three refusals, and they do not reach the same acts
//!
//! It would be tempting to fold everything below into one `match`. It would
//! also be wrong, because the checks have different domains:
//!
//! 1. **Terminality** ([`check_head_write`]) reaches **every head write** — a
//!    save and a publish as much as a transition (P-D-25, widened by P-D-32).
//!    A save on a `retired` row never asks a question about an edge, so an
//!    edge-keyed check would let it through. It is a separate function for
//!    exactly that reason: the save and publish doors call it without calling
//!    [`guard`] at all.
//! 2. **The edge list** ([`ADMITTED_EDGES`]) reaches a `lifecycle_state`
//!    change and nothing else.
//! 3. **A same-value write is not a transition** and must not be refused by
//!    either. The head row is the authoring surface in every non-terminal
//!    state (H1 fix), so a save on a `published` head presents
//!    `published -> published`, and a re-publish takes no edge at all
//!    (`inst-fd-publish-freeze`: *"a re-publish changes the version, never the
//!    state"*). A guard that treated the diagonal as an edge would refuse
//!    every save on a published entity.
//!
//! # The floor is policy-free, deliberately
//!
//! `inst-fd-transition-policy-free`: the two-person ceremony on
//! `deprecated -> published`, scheduled retirement, retirement cascades — all
//! of these are **conditions on legal edges**, registered by slices 04 and 05
//! as validators on the edge. None of them is in this module and none should
//! be added to it: an edge either exists in the machine or it does not, and
//! whether a particular actor may take it today is a different question,
//! asked in a different phase. A policy compiled into this floor could not be
//! registered, unregistered, or reasoned about per tenant.
//!
//! # This module emits nothing
//!
//! `inst-fd-transition-events`: there is **no event** on
//! `published -> deprecated`, `deprecated -> published` or
//! `deprecated -> retired`; slice 04 announces those three (except the
//! `Product` side of `deprecated -> retired`, which no slice announces, §4.5).
//! `draft -> discarded` emits `SkuDiscarded`/`ProductDiscarded` through the
//! discard door (`inst-fd-discard`), not from here. So this module enqueues
//! nothing, and a later reader tempted to add an emit here should add it to
//! the owning door instead.
//!
//! # What this module does **not** ship
//!
//! The approval-invalidation hook is a port ([`ApprovalInvalidationHook`])
//! with a no-op default ([`NoApprovalStoreHook`]), for the same reason
//! [`crate::domain::governance::NoMaterialityPolicyGate`] exists: there is no
//! approval store at this commit, so there is nothing to invalidate. A hook
//! that failed closed here would refuse every ordinary transition the gear can
//! currently take, against a store that does not exist. The real hook is owed
//! to slice 05.
//!
//! There is also no door here. Applying a [`TransitionDecision`] — running the
//! bump, calling the hook, writing the row — belongs to the transition and
//! discard doors, which are a later slice's.

use bss_products_sdk::models::LifecycleState;
use toolkit_macros::domain_model;

use crate::domain::error::DomainError;
use crate::domain::governance::EntityRef;

/// The complete edge list (`inst-fd-transition-edges`). Anything outside it,
/// off the same-value diagonal, is `ILLEGAL_TRANSITION`.
///
/// A `const` array rather than a `match` arm set because it is also the thing
/// a test quantifies over: the machine's shape is data, so a sixth edge is
/// visible as a data change and cannot hide inside control flow. The physical
/// layer states the same set independently, as the head-row trigger's
/// `lifecycle_state` whitelist — two enforcements of one rule, which is the
/// gear's posture everywhere.
pub const ADMITTED_EDGES: [(LifecycleState, LifecycleState); 5] = [
    // The publish door's own edge. It is the door that owns it, not a
    // transition door: see `GATED_EDGES` below.
    (LifecycleState::Draft, LifecycleState::Published),
    // The discard door's. Releases the `skuCode`/`productCode` reservation by
    // the same write, the reservation indexes excluding `discarded` rows.
    (LifecycleState::Draft, LifecycleState::Discarded),
    (LifecycleState::Published, LifecycleState::Deprecated),
    // Un-deprecate. The two-person condition on it is slice 05's validator,
    // not this floor's.
    (LifecycleState::Deprecated, LifecycleState::Published),
    (LifecycleState::Deprecated, LifecycleState::Retired),
];

/// The edges whose act **consumes an approval in the same transaction**, and
/// which therefore bump once with no hook (P-D-26, extended by P-D-34).
///
/// Today this is `draft -> published` alone — the edge the publish door owns.
/// P-D-30 puts the gate phase on further edges and P-D-34 extends the same
/// exception to every one of them; when slice 05 lands those, they are added
/// here rather than to a second rule somewhere else, which is why this is a
/// named set and not an `if from == Draft` inside [`effects_for`].
const GATED_EDGES: [(LifecycleState, LifecycleState); 1] =
    [(LifecycleState::Draft, LifecycleState::Published)];

/// Who performs the `internal_revision += 1` an admitted transition owes.
///
/// The row bumps **once** either way — see [`TransitionEffects::bumps_on_the_row`].
/// What differs is whether the guard's caller owes a bump of its own, and
/// getting that backwards is exactly the defect `inst-fd-publish-bump` guards
/// against: the publish door is *one act*, not a transition plus a publish, so
/// a transition-owned bump on top of the door's own head-row `UPDATE` would
/// bump twice and break the `ETag` contract the M-2 fix depends on.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevisionBump {
    /// The transition performs its own bump, exactly as a save does.
    Own,
    /// The bump is carried by the authorized act's own head-row `UPDATE` —
    /// the publish door's single statement, which also writes
    /// `lifecycle_state`, `published_version` and `composition_pending`. The
    /// guard's caller adds none of its own.
    CarriedByTheAuthorizedAct,
}

/// Whether the transition fires the approval-invalidation hook.
///
/// An enum rather than a `bool` field because the two answers have reasons
/// that a `true`/`false` cannot carry, and because a caller reading
/// `effects.invalidation` at a call site should not have to remember which way
/// round the flag ran.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalInvalidation {
    /// Fire it: the head at revision N has moved, so any approval snapshot
    /// pinned at N no longer describes the row (M-2 fix, slice-05 review).
    Fire,
    /// Skip it: this act **consumes** an approval in the same transaction, and
    /// a hook firing against the record the act is spending has no defined
    /// ordering (05 C3; P-D-30 reproduced the same collision on
    /// `deprecated -> published`).
    Skip,
}

/// What an admitted transition costs the row, returned by [`guard`] so a
/// caller cannot get it wrong by forgetting.
///
/// `inst-fd-transition-bump` is a rule with an exception, and an exception
/// left as prose is one a door eventually misses. Returning the decision means
/// the door reads the answer rather than re-deriving it from the edge.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransitionEffects {
    /// Who performs the revision bump.
    pub revision_bump: RevisionBump,
    /// Whether the approval-invalidation hook fires.
    pub invalidation: ApprovalInvalidation,
}

impl TransitionEffects {
    /// How many bumps the **guard's caller** owes: one for an ordinary
    /// transition, none for a gated edge whose act already carries it.
    ///
    /// **No production caller yet** — every call site is a test or
    /// [`Self::bumps_on_the_row`]. Both doors that take an edge today carry
    /// their bump in a statement they were already writing, so neither has to
    /// ask. The first reader is expected to be slice 04's transition door,
    /// which drives an edge it does not own a head-row `UPDATE` for and must
    /// therefore be told how many bumps to add.
    #[must_use]
    /// **No production caller yet, measured 2026-08-30** - see
    /// [`TransitionEffects::bumps_on_the_row`] for what the three counters
    /// are for and which slice is expected to read them first.
    pub const fn bumps_the_guard_owns(self) -> u8 {
        match self.revision_bump {
            RevisionBump::Own => 1,
            RevisionBump::CarriedByTheAuthorizedAct => 0,
        }
    }

    /// How many bumps the **authorized act's own statement** carries: one on
    /// a gated edge, none on an ordinary transition, which has no such
    /// statement to ride in.
    ///
    /// **No production caller yet** — it exists as the other half
    /// [`Self::bumps_on_the_row`] sums, and as the operand a door checking its
    /// own statement against the floor would read. The first reader is expected
    /// to be slice 05, when a gated edge beyond `draft -> published` lands
    /// (P-D-30) and the question stops having one answer.
    #[must_use]
    /// **No production caller yet, measured 2026-08-30** - see
    /// [`TransitionEffects::bumps_on_the_row`].
    pub const fn bumps_the_authorized_act_carries(self) -> u8 {
        match self.revision_bump {
            RevisionBump::Own => 0,
            RevisionBump::CarriedByTheAuthorizedAct => 1,
        }
    }

    /// How many times `internal_revision` moves on the row for this
    /// transition: **one**, on every admitted edge, gated or not.
    ///
    /// Summed from the two halves rather than returned as the constant `1`,
    /// so it *computes* the property `inst-fd-publish-bump` pins instead of
    /// asserting it. A later variant that let both the guard and the act bump
    /// would show up here as `2` — which is exactly the double-bump P-D-26's
    /// "once" forbids — rather than being hidden behind a literal.
    ///
    /// **No production caller yet**: it is asserted by
    /// `transition_tests::every_admitted_edge_bumps_the_row_exactly_once` and
    /// read by no door. It is kept rather than deleted because it is the
    /// executable form of `inst-fd-publish-bump`'s "once", and the first reader
    /// is expected to be slice 05's approval store, whose pinned revision is
    /// only sound while this stays `1`.
    #[must_use]
    /// **No production caller yet, measured 2026-08-30.** Every call site of
    /// this and its two companions is a test or another of the three.
    ///
    /// What actually enforces "**once**" today is elsewhere and in two
    /// places: the head-row trigger admits `internal_revision` only as
    /// `OLD + 1`, and each door issues exactly one head-row `UPDATE`. This
    /// arithmetic is the *third* statement of the same rule, and it is the
    /// one a future variant would surface in - a decision that let both the
    /// guard and the act bump would read `2` here, which is what P-D-26's
    /// "once" forbids, instead of hiding behind a literal.
    ///
    /// The first reader is expected to be a transition door - the floor has
    /// none of its own, `draft -> discarded` being the publish-phase discard
    /// door's and the other four edges belonging to slices 04 and 05.
    pub const fn bumps_on_the_row(self) -> u8 {
        self.bumps_the_guard_owns() + self.bumps_the_authorized_act_carries()
    }
}

/// What [`guard`] decided about a head write's `from`/`to` pair.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionDecision {
    /// `from == to`: a save, or a re-publish, on a non-terminal head. Not a
    /// transition, so it carries none of a transition's effects — the writing
    /// door supplies its own bump and its own hook, as a save always has.
    NotATransition,
    /// An admitted edge, with what it costs the row.
    Transition(TransitionEffects),
}

/// The terminality check, which reaches **every** head write and not only a
/// transition (`inst-fd-terminal`, P-D-25 widened by P-D-32).
///
/// Called on its own by the save, publish and correction doors; called first
/// by [`guard`]. The physical layer refuses the same writes independently, the
/// append-only trigger's whitelist admitting no `lifecycle_state` write out of
/// `retired` or `discarded`.
///
/// # Errors
///
/// [`DomainError::EntityTerminal`] where the head is `retired` or
/// `discarded`. The message names the state, so a door's answer says which
/// terminal state refused it rather than only that some state did.
pub fn check_head_write(from: LifecycleState) -> Result<(), DomainError> {
    if from.is_terminal() {
        return Err(DomainError::EntityTerminal(format!(
            "no head write is admitted on a {} entity",
            from.as_str()
        )));
    }
    Ok(())
}

/// The transition guard: terminality, then the same-value case, then the edge
/// list (`inst-fd-transition-guard`).
///
/// The order is not cosmetic. Terminality first, because a write on a
/// `retired` row is refused whatever it asked for, and reporting
/// `ILLEGAL_TRANSITION` there would name the wrong rule; the same-value case
/// second, because the diagonal is not in [`ADMITTED_EDGES`] and would
/// otherwise fall through to the edge list and be refused.
///
/// # Errors
///
/// - [`DomainError::EntityTerminal`] where `from` is terminal — see
///   [`check_head_write`].
/// - [`DomainError::IllegalTransition`] where `from != to` and the pair is not
///   in [`ADMITTED_EDGES`], naming both states as the wire codes spell them.
pub fn guard(from: LifecycleState, to: LifecycleState) -> Result<TransitionDecision, DomainError> {
    check_head_write(from)?;

    if from == to {
        return Ok(TransitionDecision::NotATransition);
    }

    if ADMITTED_EDGES.contains(&(from, to)) {
        Ok(TransitionDecision::Transition(effects_for(from, to)))
    } else {
        Err(DomainError::IllegalTransition {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
        })
    }
}

/// `inst-fd-transition-bump` applied to one admitted edge.
///
/// Private: the effects of an edge are not a question a caller should be able
/// to ask about a pair [`guard`] has not admitted.
fn effects_for(from: LifecycleState, to: LifecycleState) -> TransitionEffects {
    if GATED_EDGES.contains(&(from, to)) {
        TransitionEffects {
            revision_bump: RevisionBump::CarriedByTheAuthorizedAct,
            invalidation: ApprovalInvalidation::Skip,
        }
    } else {
        TransitionEffects {
            revision_bump: RevisionBump::Own,
            invalidation: ApprovalInvalidation::Fire,
        }
    }
}

/// Whether a head write fires the approval-invalidation hook, for **either**
/// answer [`guard`] can give.
///
/// [`TransitionEffects::invalidation`] answers the edge case and this answers
/// the whole of [`TransitionDecision`], because the diagonal is a case of the
/// same rule and not a case outside it. Every other part of that rule —
/// [`ADMITTED_EDGES`], `GATED_EDGES`, [`effects_for`] — lives in this module,
/// and a door left to fold in the [`TransitionDecision::NotATransition`] arm
/// at its own call site is a door that can disagree with its neighbour about
/// it. Two of them did.
///
/// # The `NotATransition` arm, and why it is `Skip`
///
/// `inst-fd-transition-bump` is a rule with an exception: every frozen-content
/// write bumps `internal_revision` and fires the hook, **except** *"a
/// transition that consumes an approval in the same transaction"*, which bumps
/// once and fires no hook (05 C3; P-D-30 put the gate phase on further edges
/// and P-D-34 extended the exception to all of them).
///
/// A **re-publish** is such an act. It reaches this arm rather than
/// [`TransitionDecision::Transition`] because `inst-fd-publish-freeze` makes it
/// *"change the version, never the state"* — a `published` head presents
/// `published -> published` and takes no edge — but it is still a publish, and
/// 05 `inst-gv-materiality` still gives it an `ApprovalRecord` (a bucket-iv-only
/// re-publish is the non-material case, `required = min(N, 1)`, not the
/// no-record case). The transaction that writes version `N + 1` is therefore
/// the transaction that consumes that record, and the exception's own
/// justification applies unchanged: a hook firing against the record the act is
/// consuming has no defined ordering.
///
/// # What this does not answer
///
/// A **save** also presents `from == to` and also lands on this arm, and a save
/// consumes no approval — 05 C3 fires the hook on it. That is not a
/// contradiction here because a save's hook is not read off this function: the
/// save door owns its own bump and its own hook, as [`TransitionDecision`]'s
/// own doc says, and calls neither [`guard`] nor this. A later save-side caller
/// that wanted an answer from this module would need a decision distinguishing
/// the two same-value acts, which the current [`TransitionDecision`] cannot
/// carry — and that is a widening of the type, not a new arm here.
#[must_use]
pub const fn invalidation_for(decision: TransitionDecision) -> ApprovalInvalidation {
    match decision {
        TransitionDecision::Transition(effects) => effects.invalidation,
        TransitionDecision::NotATransition => ApprovalInvalidation::Skip,
    }
}

/// The approval-invalidation hook a transition fires when
/// [`TransitionEffects::invalidation`] is [`ApprovalInvalidation::Fire`].
///
/// A port for the same reason the governance gate is one: the records it
/// invalidates are slice 05's and do not exist here. The Foundation knows only
/// that a moved head invalidates whatever was pinned to its old revision; what
/// "invalidate" does to a record is 05's.
pub trait ApprovalInvalidationHook {
    /// Invalidate every approval pinned to this subject's pre-transition
    /// revision.
    ///
    /// # Errors
    ///
    /// [`DomainError`] where the hook could not do its work. It runs inside
    /// the transition's own transaction, so a failure fails the transition
    /// rather than leaving a stale approval standing against a moved head.
    fn invalidate(&self, subject: EntityRef) -> Result<(), DomainError>;
}

/// The hook the gear runs until slice 05 supplies an approval record store.
///
/// A no-op that **succeeds**, not one that fails closed. There is no store, so
/// there is no record that could be stale; failing closed here would refuse
/// every ordinary transition the gear can currently take, on behalf of rows
/// that do not exist. Once 05 lands its store this type has no callers left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoApprovalStoreHook;

impl ApprovalInvalidationHook for NoApprovalStoreHook {
    /// # Errors
    ///
    /// Never: there is no store to fail against.
    fn invalidate(&self, _subject: EntityRef) -> Result<(), DomainError> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod transition_tests;
