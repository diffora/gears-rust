//! The `GovernedLiveOp` envelope — what slice 05 approves for a **live-entity**
//! mutation, and what the apply step re-validates
//! (`design/02-taxonomy-attributes.md` §3.4 `inst-gl-envelope` and
//! `inst-gl-atomic`; **P-D-21**, **P-D-93**).
//!
//! # It pins the operation, not a revision
//!
//! `inst-gl-envelope`: *"A `GovernedLiveOp` pins the **operation** (kind +
//! target + payload + the target's expected current state), **not** an entity
//! revision"*. That is the whole reason this type exists beside the
//! Foundation's pinned-revision publish: a category, an attribute definition
//! and a recognized-set member have **no `internal_revision`** — they are live
//! rows, not versioned entities — so there is no revision to pin and the
//! expected **state** takes its place.
//!
//! The apply step re-validates that expected state against the live row and
//! fails **`STALE_LIVE_OP`** if the world moved. This is the live-entity
//! analogue of `STALE_REVISION`, and the two must not be confused: 02 §3.5
//! says so in as many words while introducing a *third* code —
//! `STALE_CATEGORY_TOKEN` — for the category live-value door's own
//! precondition, which is neither of them.
//!
//! # Apply is atomic, and the currency check is inside that atomicity
//!
//! `inst-gl-atomic`: *"mutation + event in one transaction (P-D-21); there is
//! no partially-applied taxonomy op"*. [`GovernedLiveOp::apply`] takes the
//! mutation as a closure and runs the currency check **immediately before**
//! it, so a caller holding a transaction cannot interleave a commit between
//! the two — the check and the write are one step from the caller's side.
//! Enqueueing the event belongs to the same closure for the same reason.
//!
//! # Slice 03 reuses this type without redefining it
//!
//! `dod-governed-live-op` requires that the type *"be exported for
//! `03-sku-classification` to reuse without redefinition"*, and
//! `design/03` §3.1's own step says its four sets' mutations *"ride
//! `GovernedLiveOp` (02 §3.1)"*. So the op **kind** is an open string rather
//! than an enum of 02's operations: an enum would make every 03 mutation a
//! change to 02's type, which is the redefinition the `DoD` forbids in a
//! different spelling.
//!
//! # What is absent, and why the `DoD` is NOT ticked
//!
//! `dod-governed-live-op` also requires that the envelope be **submitted to
//! the `05-governance` gate**, and that half is **not built** — for a reason
//! **P-D-93** wrote down as its own first residue before this module existed:
//! `GovernanceGate::evaluate` takes an `EntityRef` and an
//! `InternalRevision`, and *"a live op whose subject is **not** an entity
//! would need a second contract and this decision does not grant one"*. A
//! category, a definition and a set member have no revision — that absence is
//! the very reason this envelope pins a **state** instead — so submitting one
//! through today's contract would mean inventing a mapping from a live target
//! to an entity ref, which is that second contract written here rather than
//! decided.
//!
//! So this module ships the three halves that need no gate: the envelope, the
//! currency check with its own code, and the atomic apply. The submission
//! half waits on either 05's own submit door (05 §7 row 12 — no route
//! declared) or a gate contract for non-entity subjects, and **no reading of
//! the green suite here should be taken for it.**

use crate::domain::error::DomainError;

/// One live-entity mutation, pinned for approval and for re-validation.
///
/// `S` is the caller's own expected-state type — a category's `state`, a set
/// member's, a definition's — compared by equality at apply. Generic rather
/// than a shared enum for the reason the module doc gives: an enum would put
/// every slice's operations in 02's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GovernedLiveOp<S> {
    /// The operation, in the owning slice's own vocabulary
    /// (`category.reparent`, `recognized_set.deprecate`, …). An open string:
    /// see the module doc on why this is not an enum.
    pub kind: String,
    /// What the operation acts on, rendered so an approval record can carry
    /// it as a subject ref.
    pub target: String,
    /// The operation's own arguments, canonically rendered by the caller —
    /// this type never inspects it, and an approval pins exactly these bytes.
    pub payload: String,
    /// The target's state as the submitter saw it. Re-validated at apply.
    pub expected_state: S,
}

impl<S: PartialEq + core::fmt::Debug> GovernedLiveOp<S> {
    /// Apply the operation, the currency check running immediately before the
    /// mutation so the two are one step from the caller's side
    /// (`inst-gl-atomic`).
    ///
    /// The closure owns the row write **and** the event enqueue, which is what
    /// makes *"no partially-applied taxonomy op"* structural rather than a
    /// convention: a caller never holds a point between them at which it could
    /// commit one alone.
    ///
    /// # Errors
    ///
    /// [`DomainError::StaleLiveOp`] when the pinned state no longer holds — in
    /// which case the closure is **not** run — or whatever the closure itself
    /// returns.
    pub fn apply<T, F>(&self, live_state: &S, mutate: F) -> Result<T, DomainError>
    where
        F: FnOnce() -> Result<T, DomainError>,
    {
        self.check_still_current(live_state)?;
        mutate()
    }

    /// Re-validate the pinned state against the live row.
    ///
    /// # Errors
    ///
    /// [`DomainError::StaleLiveOp`] when the live state differs — the world
    /// moved between submission and apply, and the approval was given against
    /// a state that no longer holds.
    pub fn check_still_current(&self, live_state: &S) -> Result<(), DomainError> {
        if &self.expected_state == live_state {
            return Ok(());
        }
        Err(DomainError::StaleLiveOp(format!(
            "{}: the {} was {:?} at submission and is {:?} now",
            self.kind, self.target, self.expected_state, live_state
        )))
    }
}

#[cfg(test)]
#[path = "live_op_tests.rs"]
mod live_op_tests;
