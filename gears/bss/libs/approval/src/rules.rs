//! Pure quorum and separation-of-duties rules for the current generation.

use crate::model::{ApprovalError, Decision, ItemRef, Unit, UnitState, Verdict};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproveStep {
    NeedMore { have: u32, need: u32 },
    Apply,
}

/// Distinct authors of a unit's items.
#[must_use]
pub fn authors_of(items: &[ItemRef]) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = items.iter().map(|i| i.created_by).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The submitter and every item author are excluded from approving.
///
/// # Errors
/// Returns [`ApprovalError::SodViolation`] if the actor submitted the unit or authored an item.
pub fn check_sod(unit: &Unit, actor: Uuid, authors: &[Uuid]) -> Result<(), ApprovalError> {
    if actor == unit.submitted_by || authors.contains(&actor) {
        return Err(ApprovalError::SodViolation);
    }
    Ok(())
}

/// Did `actor` already vote in the unit's **current** generation?
#[must_use]
pub fn already_voted(unit: &Unit, decisions: &[Decision], actor: Uuid) -> bool {
    decisions
        .iter()
        .any(|d| d.actor == actor && d.generation == unit.generation)
}

/// Whether one actor may approve a unit, and the votes the quorum counts: the approve eligibility
/// of [`approve_eligibility`], the one rule the engine's approve and every reader judge by.
#[derive(Debug)]
pub struct ApproveEligibility {
    /// The approve votes that count toward the quorum: of the unit's current generation and not
    /// stale. A reject, a stale vote and a vote of an earlier generation do not count.
    pub approvals: u32,
    /// `None` when the actor may approve; else the refusal the engine answers its approve with:
    /// [`ApprovalError::AlreadyDecided`] for a terminal unit, [`ApprovalError::SodViolation`] for
    /// its submitter or an item author, [`ApprovalError::DuplicateVote`] for an actor who already
    /// voted in the current generation, in that order.
    pub refusal: Option<ApprovalError>,
}

/// The approve eligibility of `actor` on `unit`, from the unit, its stored items (the current
/// generation's: a stale refresh rewrites them) and its decisions of every generation. Pure, and
/// the engine's own rule: [`crate::Engine::approve`] judges through it once it has loaded the unit
/// at the reviewer's generation, so a reader that shows the counts or whether its reader may
/// approve cannot drift from the vote door.
#[must_use]
pub fn approve_eligibility(
    unit: &Unit,
    items: &[ItemRef],
    decisions: &[Decision],
    actor: Uuid,
) -> ApproveEligibility {
    let counted = decisions
        .iter()
        .filter(|d| d.verdict == Verdict::Approve && !d.stale && d.generation == unit.generation)
        .count();
    let refusal = if unit.state == UnitState::Pending {
        check_sod(unit, actor, &authors_of(items))
            .err()
            .or_else(|| {
                already_voted(unit, decisions, actor).then_some(ApprovalError::DuplicateVote)
            })
    } else {
        Some(ApprovalError::AlreadyDecided)
    };
    ApproveEligibility {
        approvals: u32::try_from(counted).unwrap_or(u32::MAX),
        refusal,
    }
}

/// What an approve vote by `actor` does, given the votes cast so far: [`approve_eligibility`]'s
/// refusal, else the vote pends or applies. Pure: the caller has loaded the unit, its stored
/// items and its decisions inside the transaction. Decisions must belong to this unit, with at
/// most one per actor per generation.
///
/// # Errors
/// Returns [`ApprovalError::AlreadyDecided`] for a terminal unit,
/// [`ApprovalError::SodViolation`] for its submitter or an item author, or
/// [`ApprovalError::DuplicateVote`] for an actor who already voted in this generation.
pub fn evaluate_approve(
    unit: &Unit,
    decisions: &[Decision],
    actor: Uuid,
    items: &[ItemRef],
) -> Result<ApproveStep, ApprovalError> {
    let judged = approve_eligibility(unit, items, decisions, actor);
    if let Some(refusal) = judged.refusal {
        return Err(refusal);
    }
    let have = judged.approvals.saturating_add(1);
    if have >= unit.quorum_required {
        Ok(ApproveStep::Apply)
    } else {
        Ok(ApproveStep::NeedMore {
            have,
            need: unit.quorum_required,
        })
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
