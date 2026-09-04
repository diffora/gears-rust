//! The deprecation act's rules — provenance, the cascade's per-child
//! disposition, and what an un-deprecation may reverse
//! (`design/04-lifecycle.md` §2 `inst-lc-deprecate` and
//! `inst-lc-provenance-reversal`).
//!
//! # Why provenance is a type and not a `bool`
//!
//! `deprecation_provenance` is `direct|cascaded` and the column is nullable,
//! so three states exist on the row: never deprecated, deprecated by an
//! operator, deprecated by a parent. A `bool` can carry two of those, and the
//! third is exactly the one every rule here reads — an un-deprecation
//! reverses `cascaded` children and leaves `direct` ones deprecated, so a
//! column that could not tell them apart would make `dod-provenance-reversal`
//! unimplementable.
//!
//! # The three rules, and the one shared reason they exist
//!
//! [`stamp_for`] answers what an act writes; [`disposition_for`] answers what
//! a parent's cascade does to one child; [`reversal_admits`] answers which
//! children a reversal touches. All three key on the same pair — the child's
//! `lifecycle_state` and its stored provenance — and the design states them
//! as one instruction, so they live in one module rather than being folded
//! into whichever door happened to need each first.
//!
//! **An already-`deprecated` entity is never re-stamped.** That is not a
//! tidiness rule. `direct` re-stamped as `cascaded` would make a parent's
//! un-deprecation revive exactly the child AC #17 says it must not. The
//! floor does not refuse the diagonal — `transition::guard` answers
//! `NotATransition` on `deprecated → deprecated`, its own rule 3 — so this
//! module's `None` is what a door turns into its refusal.
//!
//! # What this module does not decide
//!
//! Nothing here reads or writes a row. The physical stamp rides the same
//! `UPDATE` as the `lifecycle_state` change because the head guard's
//! row-image predicate admits it on no other terms
//! (`m20260829_000002`/`000003`), and that pairing is the repository's to
//! keep — this module only says which value the statement carries.
//!
//! # The `DoD`s this reaches, and why none of them is claimed
//!
//! Four definitions read on this module — `dod-deprecation-cascade`,
//! `dod-deprecation-provenance`, `dod-provenance-reversal` and
//! `dod-no-orphan` — and all four markers below are **bare**, not canonical.
//! Two reasons, one per pair:
//!
//! - The cascade and the provenance stamp are performed, but **no design
//!   document declares an entity-scoped door for the operator act**. Three
//!   `{products|skus}/{id}/<act>` spans are declared across the set —
//!   `publish`, `discard` and `clone` — and `deprecate` is not among them,
//!   though `design/04` §2 `inst-lc-deprecate` is a `p1` instruction
//!   describing the act; the one declared carrier, `09`'s
//!   `POST .../bulk/lifecycle`, is batch-only. The route the crate registers
//!   is therefore the crate's own, and a tick asserting it would assert a
//!   wire contract nothing in the set backs. `features/lifecycle.md` §7
//!   carries the question.
//! - The reversal and the no-orphan invariant are **rules with no act**:
//!   un-deprecation's door is `dod-undeprecation`'s, blocked on §7 row 32,
//!   and the retirement flip needs `products_scheduled_transition`, which
//!   does not ship. Both rules and their probes are here so the act that
//!   lands later consults one answer instead of inventing a second.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-deprecation-cascade:p1
//! @cpt-dod:cpt-cf-bss-products-dod-deprecation-provenance:p1
//! @cpt-cf-bss-products-dod-provenance-reversal
//! @cpt-cf-bss-products-dod-no-orphan

use bss_products_sdk::models::LifecycleState;

use super::error::DomainError;

/// Why an entity is `deprecated` — the `deprecation_provenance` column's two
/// admitted values. `design/01` annotates the column `direct|cascaded` in
/// both of its table shapes (§4.1 and §4.2); **no `CHECK` pins it**, so the
/// pair is application-enforced — this parse, and the repository's
/// `parse_provenance` on every read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provenance {
    /// An operator deprecated this entity itself.
    Direct,
    /// A parent's deprecation cascaded onto this child.
    Cascaded,
}

impl Provenance {
    /// The stored spelling, which is also the wire spelling. No database
    /// constraint backs it; the repository's read-side parse is what refuses
    /// a stored value outside the pair.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Cascaded => "cascaded",
        }
    }

    /// Parse a stored value, `None` for anything outside the pair.
    ///
    /// Fail-closed like every other roster parse in the gear: an
    /// unrecognised provenance is a corrupt row, not a `direct`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "cascaded" => Some(Self::Cascaded),
            _ => None,
        }
    }
}

/// What a parent's cascade does to one child, by the child's own state
/// (`design/04` §2 `inst-lc-deprecate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildDisposition {
    /// `published` — deprecate it, stamping [`Provenance::Cascaded`].
    Deprecate,
    /// Already `deprecated` — **left untouched**, its provenance never
    /// re-stamped.
    LeaveUntouched,
    /// `draft` — **skipped and listed**, never transitioned. The floor admits
    /// no `draft → deprecated` edge, and any failure rejects the whole
    /// mutation (`01 inst-fd-fail-closed`), so treating a draft child as part
    /// of the population would make deprecating a Product with one draft SKU
    /// fail `ILLEGAL_TRANSITION` with no remedy available to the operator.
    SkipAndList,
    /// `retired` or `discarded` — terminal, and outside the population
    /// rather than skipped inside it.
    OutsidePopulation,
}

/// The per-child disposition a plain parent deprecation takes.
#[must_use]
pub const fn disposition_for(child: LifecycleState) -> ChildDisposition {
    match child {
        LifecycleState::Published => ChildDisposition::Deprecate,
        LifecycleState::Deprecated => ChildDisposition::LeaveUntouched,
        LifecycleState::Draft => ChildDisposition::SkipAndList,
        LifecycleState::Retired | LifecycleState::Discarded => ChildDisposition::OutsidePopulation,
    }
}

/// The provenance an act writes, or `None` where it writes nothing.
///
/// `None` is the already-`deprecated` case and is not an error: the design
/// has the retirement arm take no transition on a SKU that is already
/// `deprecated`, *"the event firing only where the transition is taken"*, so
/// a caller reading `None` skips the statement and the event together.
///
/// # Errors
///
/// [`DomainError::EntityTerminal`] on a terminal head, naming the state —
/// the same answer `transition::check_head_write` gives, raised here so a
/// caller that asks this question first cannot get a provenance for a row no
/// write is admitted on. [`DomainError::IllegalTransition`] on a `draft`
/// head, the floor admitting no `draft → deprecated` edge.
pub fn stamp_for(from: LifecycleState, act: Provenance) -> Result<Option<Provenance>, DomainError> {
    match from {
        LifecycleState::Deprecated => Ok(None),
        LifecycleState::Published => Ok(Some(act)),
        // A `draft` reaches here only through a caller that did not consult
        // `disposition_for` first. The floor's edge list is the authority and
        // refuses it; answering `None` here would silently turn a refusal
        // into a no-op.
        LifecycleState::Draft => Err(DomainError::IllegalTransition {
            from: LifecycleState::Draft.as_str().to_owned(),
            to: LifecycleState::Deprecated.as_str().to_owned(),
        }),
        // The terminal arm answers directly rather than via an
        // `is_terminal()` prelude and a panic on the leftover states: two
        // sibling doors document `unreachable!` as a posture this crate
        // avoids, and an arm that returns is one a refactor cannot turn into
        // a panic path.
        LifecycleState::Retired | LifecycleState::Discarded => Err(DomainError::EntityTerminal(
            format!("no deprecation is admitted on a {} entity", from.as_str()),
        )),
    }
}

/// Whether an un-deprecation of the parent reverses this child
/// (`inst-lc-provenance-reversal`).
///
/// **Only `cascaded`.** A child an operator deprecated directly stays
/// deprecated through its parent's reversal, the provenance column being the
/// operand — which is the whole reason the column is not a `bool` and the
/// whole reason a re-stamp is refused.
///
/// A child with **no** stored provenance is not reversed either: a
/// `deprecated` row that names no cause is one this gear did not deprecate
/// through either path, and reviving it would be inventing a reversal for an
/// act with no record.
#[must_use]
pub const fn reversal_admits(stored: Option<Provenance>) -> bool {
    matches!(stored, Some(Provenance::Cascaded))
}

/// The no-orphan invariant: no `published` child may sit under a `retired`
/// parent (`dod-no-orphan`, AC on `flow-retire-product`).
///
/// # Why this takes the child states rather than reading them
///
/// The invariant has to be re-checked **at the flip**, not only planned at
/// confirmation, and the flip's transaction is the only place that holds the
/// child rows as they are at that instant. A function that read them itself
/// would read them on some other connection at some other moment, which is
/// precisely the staleness the *"re-checked at flip"* clause exists to
/// refuse.
///
/// Returns `true` when the flip may proceed. A `published` child is a
/// **deferral**, not a wire refusal (**P-D-113** arm 5): the runner is not
/// a door, so no `DomainError` is minted. The `outcome_reason` is
/// [`crate::domain::retention::RetentionHold::REASON`].
#[must_use]
pub fn no_orphan_at_flip(children: &[LifecycleState]) -> bool {
    !children.contains(&LifecycleState::Published)
}

#[cfg(test)]
#[path = "deprecation_tests.rs"]
mod deprecation_tests;
