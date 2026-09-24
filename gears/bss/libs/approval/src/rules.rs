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

/// What an approve vote by `actor` does, given the votes cast so far. Pure: the
/// caller has loaded the unit and its decisions inside the transaction.
/// Decisions must belong to this unit, with at most one per actor per generation.
///
/// # Errors
/// Returns [`ApprovalError::AlreadyDecided`] for a terminal unit,
/// [`ApprovalError::SodViolation`] for its submitter or an item author, or
/// [`ApprovalError::DuplicateVote`] for an actor who already voted in this generation.
pub fn evaluate_approve(
    unit: &Unit,
    decisions: &[Decision],
    actor: Uuid,
    authors: &[Uuid],
) -> Result<ApproveStep, ApprovalError> {
    if unit.state != UnitState::Pending {
        return Err(ApprovalError::AlreadyDecided);
    }
    check_sod(unit, actor, authors)?;
    if already_voted(unit, decisions, actor) {
        return Err(ApprovalError::DuplicateVote);
    }
    let counted = decisions
        .iter()
        .filter(|d| d.verdict == Verdict::Approve && !d.stale && d.generation == unit.generation)
        .count();
    let have = u32::try_from(counted).unwrap_or(u32::MAX).saturating_add(1);
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
