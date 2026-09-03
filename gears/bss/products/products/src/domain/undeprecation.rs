//! Un-deprecation policy: the live-retire-intent guard and which children
//! a reversal touches (`dod-undeprecation`, `dod-provenance-reversal`).
//!
//! The reversal operand itself is [`super::deprecation::reversal_admits`].
//! This module adds the `RETIREMENT_PENDING` check that looking only at the
//! subject's own intent missed: a parent's cancel-then-un-deprecate revived
//! `cascaded` children whose retire intents stayed live.
//!
//! The `DomainError` arm is a D7 patch; continuations return
//! [`LifecycleRefusal`].
//!
//! @cpt-dod:cpt-cf-bss-products-dod-undeprecation
//!
//! **Bare on purpose — the tick was withdrawn by the lead on 2026-09-03.** `features/lifecycle.md` §7 row 32: the `DoD`'s last clause requires the governed cancel to
//! clear `replaced_by_sku_id` *"for the parent and every child leg"*, and the parent is a
//! **Product**, which has no such column — `dod-lifecycle-columns` pins it to `products_sku`
//! and §1.4 records the slice being corrected once already for listing it on both. The clause
//! is **unsatisfiable as written**, so this is a clause defeated and not a pointer (P-D-109).
//! A `:p1` marker is itself a done-claim, so it returns when the clause does.
//! @cpt-dod:cpt-cf-bss-products-dod-provenance-reversal:p1

use uuid::Uuid;

use super::deprecation::{Provenance, reversal_admits};
use super::lifecycle::LifecycleRefusal;

/// One live retire intent the un-deprecation would expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockingIntent {
    /// The entity that still holds a live retire `ScheduledTransition`.
    pub entity_id: Uuid,
}

/// Refuse un-deprecation while a live retire intent exists on the subject
/// **or on any child this reversal would revive**.
///
/// `revived` is the child set `reversal_admits` already filtered — this
/// function does not re-derive provenance. `intents` is the live retire
/// population the door prefetched (continuation: a scan, not a registered
/// rule).
///
/// # Errors
///
/// [`LifecycleRefusal`] with [`LifecycleRefusal::RETIREMENT_PENDING`],
/// naming the blocking entity ids.
pub fn refuse_if_live_retire_intents(
    subject_id: Uuid,
    revived: &[Uuid],
    intents: &[BlockingIntent],
) -> Result<(), LifecycleRefusal> {
    let mut named = Vec::new();
    for intent in intents {
        if intent.entity_id == subject_id || revived.contains(&intent.entity_id) {
            named.push(intent.entity_id);
        }
    }
    if named.is_empty() {
        return Ok(());
    }
    Err(LifecycleRefusal::retirement_pending(&named))
}

/// Children a parent un-deprecation reverses: `cascaded` only.
#[must_use]
pub fn children_the_reversal_touches(stored: &[(Uuid, Option<Provenance>)]) -> Vec<Uuid> {
    stored
        .iter()
        .filter_map(|(id, provenance)| reversal_admits(*provenance).then_some(*id))
        .collect()
}

#[cfg(test)]
#[path = "undeprecation_tests.rs"]
mod undeprecation_tests;
