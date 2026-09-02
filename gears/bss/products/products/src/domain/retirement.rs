//! Retirement initiation, the flip-guard stub, `replacedBy`, and the v1
//! EOL lockout (`dod-retirement-initiation`, `dod-flip-guard`,
//! `dod-replaced-by`, `dod-eol-lockout`, `dod-lead-window-reannounce`).
//!
//! # The flip-guard stub is the specified artifact
//!
//! Nobody is building `07-reference-signal`. The `DoD`'s last sentence is the
//! instruction: a stub predicate **MUST** be exercised in all four deferring
//! states as well as the passing one. When 07 lands it replaces
//! [`FlipPredicate`]. The seam is named in the owed register.
//!
//! # `effectiveAt` — both arms, neither authored as the answer
//!
//! §7 row 14 is open (operator input vs computed). [`effective_at`] accepts
//! an optional operator instant and compares it to `now + lead`; a missing
//! instant computes. That is a host, not a ruling.
//!
//! @cpt-cf-bss-products-dod-retirement-initiation
//! @cpt-dod:cpt-cf-bss-products-dod-flip-guard:p1
//! @cpt-cf-bss-products-dod-replaced-by
//! @cpt-cf-bss-products-dod-eol-lockout
//! @cpt-dod:cpt-cf-bss-products-dod-lead-window-reannounce:p1

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use bss_products_sdk::models::LifecycleState;

use super::lifecycle::LifecycleRefusal;

/// `07-reference-signal`'s five-state predicate, stubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipPredicate {
    /// Fresh, every producer at zero — the only passing state.
    FreshZero,
    /// Fresh, at least one producer still referencing.
    FreshPositive,
    /// Watermark older than the freshness cadence.
    Stale,
    /// No watermark has arrived.
    NeverReceived,
    /// Defensive: the registry lists no producers.
    NoProducers,
}

/// A `retirement_held` alert the stub raises on a deferring predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementHeld {
    /// Producer ids the stub names. Empty on [`FlipPredicate::NoProducers`].
    pub blocking_producers: Vec<String>,
}

impl FlipPredicate {
    /// Whether this state admits the flip.
    #[must_use]
    pub const fn admits_flip(self) -> bool {
        matches!(self, Self::FreshZero)
    }
}

/// Consult the stub. Defer on everything but fresh-zero.
///
/// # Errors
///
/// [`RetirementHeld`] naming the blocking producers in the four deferring
/// states. There is no force-retire door.
pub fn flip_guard(predicate: FlipPredicate) -> Result<(), RetirementHeld> {
    match predicate {
        FlipPredicate::FreshZero => Ok(()),
        FlipPredicate::FreshPositive => Err(RetirementHeld {
            blocking_producers: vec!["stub:fresh-positive".to_owned()],
        }),
        FlipPredicate::Stale => Err(RetirementHeld {
            blocking_producers: vec!["stub:stale".to_owned()],
        }),
        FlipPredicate::NeverReceived => Err(RetirementHeld {
            blocking_producers: vec!["stub:never-received".to_owned()],
        }),
        FlipPredicate::NoProducers => Err(RetirementHeld {
            blocking_producers: vec![],
        }),
    }
}

/// A successor named at initiation must be `published`.
///
/// # Errors
///
/// [`LifecycleRefusal::REPLACED_BY_NOT_PUBLISHED`].
pub fn replaced_by_must_be_published(
    successor: Option<LifecycleState>,
) -> Result<(), LifecycleRefusal> {
    match successor {
        None | Some(LifecycleState::Published) => Ok(()),
        Some(_) => Err(LifecycleRefusal::replaced_by_not_published()),
    }
}

/// Walk a successor chain to the first non-`retired` SKU. A cycle returns
/// the ids seen and no resolution — §7 rows 12/13 are still open; this is
/// a bounded walk, not a stored fact.
#[must_use]
pub fn resolve_replacement_chain(
    start: Uuid,
    next: &[(Uuid, Option<Uuid>, LifecycleState)],
    bound: usize,
) -> ReplacementWalk {
    let mut current = start;
    let mut seen = Vec::new();
    for _ in 0..bound {
        if seen.contains(&current) {
            return ReplacementWalk::Cycle { seen };
        }
        seen.push(current);
        let Some((_, successor, state)) = next.iter().find(|(id, _, _)| *id == current) else {
            return ReplacementWalk::End { last: current };
        };
        if *state != LifecycleState::Retired {
            return ReplacementWalk::Resolved { sku_id: current };
        }
        match successor {
            Some(next_id) => current = *next_id,
            None => return ReplacementWalk::End { last: current },
        }
    }
    ReplacementWalk::Bounded { seen }
}

/// Result of [`resolve_replacement_chain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacementWalk {
    /// First non-`retired` successor.
    Resolved {
        /// The SKU to serve.
        sku_id: Uuid,
    },
    /// Chain ended on a `retired` row with no pointer.
    End {
        /// Last id visited.
        last: Uuid,
    },
    /// `start` reached itself.
    Cycle {
        /// Ids in visit order.
        seen: Vec<Uuid>,
    },
    /// `bound` exhausted.
    Bounded {
        /// Ids visited.
        seen: Vec<Uuid>,
    },
}

/// v1 EOL lockout: flag OFF by default, `mustMigrateBy` refused.
///
/// # Errors
///
/// [`LifecycleRefusal::EOL_DISABLED`] when the field is present and the
/// flag is off.
pub fn eol_lockout(flag_on: bool, must_migrate_by_present: bool) -> Result<(), LifecycleRefusal> {
    if must_migrate_by_present && !flag_on {
        return Err(LifecycleRefusal::eol_disabled());
    }
    Ok(())
}

/// Whether a publish at `published_at` sits inside the lead window
/// `[scheduled_at, effective_at)` and must re-emit the retirement event.
#[must_use]
pub fn publish_reannounces_retirement(
    published_at: DateTime<Utc>,
    scheduled_at: DateTime<Utc>,
    effective_at: DateTime<Utc>,
) -> bool {
    published_at >= scheduled_at && published_at < effective_at
}

/// Host for row 14: compare an optional operator instant to `now + lead`.
///
/// # Errors
///
/// [`LifecycleRefusal::RETIREMENT_LEAD_TIME`] when the supplied instant is
/// earlier than the floor.
pub fn effective_at(
    now: DateTime<Utc>,
    lead: Duration,
    supplied: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, LifecycleRefusal> {
    let floor = now + lead;
    match supplied {
        None => Ok(floor),
        Some(at) if at >= floor => Ok(at),
        Some(_) => Err(LifecycleRefusal::retirement_lead_time()),
    }
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod retirement_tests;
