//! The activation runner's claim protocol and failure posture
//! (`dod-activation-runner`, `dod-runner-failure-posture`,
//! `dod-scheduled-publish-pin`).
//!
//! # No privileged path
//!
//! The runner drives foundation doors. The publish call is
//! `GateMode::PreAuthorized(approval_id)` — the mode exists so a consumed
//! record can be verified without being consumed again. Making the shipped
//! host accept that mode is strand B's `dod-preauthorized-mode`. This
//! module records the exact call; it does not widen the host.
//!
//! # The runner is its own raising door
//!
//! A door `STALE_REVISION` or `APPROVAL_REQUIRED` becomes
//! `SCHEDULE_STALE_APPROVAL` on the transition. `failed` is terminal for
//! that row. `deferred` has two populations: flip-guard (unbounded) and
//! transient dependency (bounded by `attempt_budget`).
//!
//! §7 row 8: the claim lease has no configured value. Callers pass
//! [`ClaimLease`] explicitly rather than this module minting a silent
//! default that would author the open item.
//!
//! @cpt-cf-bss-products-dod-activation-runner
//! @cpt-cf-bss-products-dod-runner-failure-posture
//! @cpt-cf-bss-products-dod-scheduled-publish-pin

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::lifecycle::LifecycleRefusal;

/// The reserved idempotency lane the runner resolves (`internal:` prefix).
pub const ACTIVATION_LANE: &str = "internal:scheduled-activation";

/// A claim lease duration. The value is the caller's — row 8 is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimLease {
    /// How long a `running` row may sit before reclaim.
    pub ttl: Duration,
}

/// Per-transition budget for the **transient** deferral arm only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptBudget {
    /// Attempts after which a transient deferral becomes `failed`.
    pub max: i32,
}

/// Why a row finished `deferred` rather than `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralPopulation {
    /// Retirement flip guard — unbounded.
    FlipGuard,
    /// Transient dependency — bounded by [`AttemptBudget`].
    TransientDependency,
}

/// What the poll does to one due row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimDecision {
    /// `pending|deferred → running`, `claimed_at` stamped.
    Claim,
    /// `running` past the lease → `pending`, `attempt += 1`.
    ReclaimLease,
    /// Not due, or another worker won the CAS.
    Skip,
}

/// The stored state the claim CAS matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredRunState {
    /// Waiting for `at`.
    Pending,
    /// Another worker holds the lease.
    Running,
    /// Flip-guard or transient hold; re-claimed on the same poll.
    Deferred,
    /// Terminal — never claimed.
    Terminal,
}

/// Decide the CAS for one row at `now`.
#[must_use]
pub fn claim_decision(
    state: StoredRunState,
    at: DateTime<Utc>,
    claimed_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    lease: ClaimLease,
) -> ClaimDecision {
    match state {
        StoredRunState::Pending | StoredRunState::Deferred if at <= now => ClaimDecision::Claim,
        StoredRunState::Running => match claimed_at {
            Some(claimed) if now >= claimed + lease.ttl => ClaimDecision::ReclaimLease,
            _ => ClaimDecision::Skip,
        },
        StoredRunState::Pending | StoredRunState::Deferred | StoredRunState::Terminal => {
            ClaimDecision::Skip
        }
    }
}

/// The idempotency key the runner writes: lane + transition id.
#[must_use]
pub fn activation_idempotency_key(transition_id: Uuid) -> String {
    format!("{ACTIVATION_LANE}:{transition_id}")
}

/// How a door refusal finishes the transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFinish {
    /// Door committed.
    Applied,
    /// Terminal for this transition.
    Failed {
        /// `outcome_reason`.
        reason: String,
    },
    /// Hold; population selects the budget rule.
    Deferred {
        /// Which arm.
        population: DeferralPopulation,
        /// `outcome_reason`.
        reason: String,
    },
}

/// Wrap a door refusal. `STALE_REVISION` and `APPROVAL_REQUIRED` become
/// `SCHEDULE_STALE_APPROVAL`. A flip-guard hold is unbounded; a transient
/// hold spends the budget.
///
/// # Errors
///
/// [`LifecycleRefusal::SCHEDULE_STALE_APPROVAL`] when the door's code is
/// one of the two wrapped codes — the runner raises, the door does not.
pub fn classify_door_refusal(
    door_code: &str,
    attempt: i32,
    budget: AttemptBudget,
    transient: bool,
) -> Result<RunFinish, LifecycleRefusal> {
    if door_code == "STALE_REVISION" || door_code == "APPROVAL_REQUIRED" {
        return Err(LifecycleRefusal::schedule_stale_approval(format!(
            "door refused {door_code}"
        )));
    }
    if door_code == "RETIREMENT_HELD" {
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: "flip guard".to_owned(),
        });
    }
    if transient {
        if attempt + 1 >= budget.max {
            return Ok(RunFinish::Failed {
                reason: format!("transient budget exhausted after {attempt} attempts"),
            });
        }
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::TransientDependency,
            reason: format!("transient: {door_code}"),
        });
    }
    Ok(RunFinish::Failed {
        reason: door_code.to_owned(),
    })
}

/// The exact gate call the runner makes. Strand B owns host acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreAuthorizedCall {
    /// The approval pinned on the `ScheduledTransition` at scheduling.
    pub approval_id: Uuid,
}

impl PreAuthorizedCall {
    /// `GateMode::PreAuthorized(approval_id)` — verify, do not consume.
    #[must_use]
    pub const fn mode_debug() -> &'static str {
        "GateMode::PreAuthorized(approval_id)"
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod activation_tests;
