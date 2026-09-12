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
//! no partially-applied taxonomy op"*. [`GovernedLiveOp::apply`] runs the
//! currency check **immediately before** the closure, so a stale op cannot
//! write — that much is structural.
//!
//! **What is NOT structural, and an earlier revision of this doc claimed was:**
//! that the closure holds both the row write and the event enqueue. `apply`
//! accepts any `FnOnce() -> Result<T, DomainError>` and cannot see what it
//! contains; a closure that writes and never enqueues compiles and passes.
//! Putting both on one transaction is the **caller's obligation** under
//! `inst-gl-atomic`, and the type enforces only check-before-mutate.
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
//! `dod-governed-live-op` also requires the envelope be **submitted to the
//! `05-governance` gate**, and that half is not built — but **not** for the
//! reason an earlier revision of this paragraph gave. It said a non-entity
//! subject *"would need a second contract and this decision does not grant
//! one"*; **P-D-67 arm 4 had granted it** the day before
//! (*"the gate's subject widens to the approval store's own pair,
//! `(subject_kind, subject_ref)`"*), and
//! [`crate::domain::governance::GateSubject::governed_live_op`] is the
//! constructor, forty lines from here. A reader steered by that sentence
//! would mint the parallel vocabulary `dod-gate-host` forbids.
//!
//! What actually remains is two things, both narrower:
//!
//! - **`evaluate`'s `expected_revision` operand.** The subject crosses the
//!   seam; the pinned revision beside it does not, because a live row has
//!   none — which is why this envelope pins a *state*. What a non-entity
//!   subject supplies there is `governance` §7 row 14, live.
//! - **`05`'s submit door**, whose route is undeclared (05 §7 row 12).
//!
//! So no test drives an envelope through a real approval record, and no
//! reading of the green suite here should be taken for one.
//!
//! # Nor through a fake one, and that is newly measured
//!
//! **P-D-93** released this `DoD`'s §7 row on three measurements, the third
//! being that *"**four** doubles ship — `RefusingGate`, `FailingGate`,
//! `RecordingGate` and `CountingRefusingGate` — and the door tests already
//! turn on which one is passed"*. They do ship, and all four are **private
//! items in two `#[cfg(test)]` door modules** — `api::rest::products_tests`
//! and `api::rest::skus_tests`, neither declared `pub` — while
//! `crate::test_support` carries no gate double at all. So the remedy that
//! measurement counted is reachable from the two doors and from nowhere else,
//! this envelope's own test module included.
//!
//! That is not an argument for re-holding the `DoD`: the row's other two
//! measurements stand, and the envelope is buildable. It is the reason the
//! apply path here is proven **against the closure it is handed** rather than
//! against a gate, and a reader who expects otherwise from P-D-93's third
//! line should know where to look. Lifting one double into `test_support` is
//! the one-file change that would close it, and that file is not this
//! strand's.

use crate::domain::error::DomainError;

/// One live-entity mutation, pinned for approval and for re-validation.
///
/// `S` is the caller's own expected-state type — a category's `state`, a set
/// member's, a definition's — compared by equality at apply. Generic rather
/// than a shared enum for the reason the module doc gives: an enum would put
/// every slice's operations in 02's type.
///
/// @cpt-dod:cpt-cf-bss-products-dod-governed-live-op:p1
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
