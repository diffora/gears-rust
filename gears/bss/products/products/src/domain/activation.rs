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
use super::retirement::RetirementHeld;

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

/// The three states a `running` row may finish in. The store writer takes
/// this rather than a raw string, so `"pending"` cannot un-finish a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledFinishState {
    /// Door committed.
    Applied,
    /// Terminal for this transition.
    Failed,
    /// Hold — flip-guard or transient.
    Deferred,
}

impl ScheduledFinishState {
    /// The stored spelling. Only these three are admitted as a finish.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Failed => "failed",
            Self::Deferred => "deferred",
        }
    }
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

impl RunFinish {
    /// The stored finish state this outcome writes.
    #[must_use]
    pub const fn state(&self) -> ScheduledFinishState {
        match *self {
            Self::Applied => ScheduledFinishState::Applied,
            Self::Failed { .. } => ScheduledFinishState::Failed,
            Self::Deferred { .. } => ScheduledFinishState::Deferred,
        }
    }
}

/// A door's own refusal, as the runner sees it. Flip-guard holds are
/// **not** a door code — they arrive as [`RetirementHeld`] and go through
/// [`defer_flip_guard`] before this classifier is ever called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorRefusal {
    /// The door's wire code (`STALE_REVISION`, `APPROVAL_REQUIRED`, …).
    pub code: &'static str,
    /// Whether this is the bounded transient-dependency arm.
    pub transient: bool,
}

/// Unbounded flip-guard hold. The runner evaluates [`super::retirement::flip_guard`]
/// itself and never turns [`RetirementHeld`] into a door-code string.
#[must_use]
pub fn defer_flip_guard(held: &RetirementHeld) -> RunFinish {
    let reason = if held.blocking_producers.is_empty() {
        "flip guard: no producers".to_owned()
    } else {
        format!("flip guard: {}", held.blocking_producers.join(", "))
    };
    RunFinish::Deferred {
        population: DeferralPopulation::FlipGuard,
        reason,
    }
}

/// Wrap a door refusal. `STALE_REVISION` and `APPROVAL_REQUIRED` become
/// `SCHEDULE_STALE_APPROVAL`. A transient hold spends the budget; everything
/// else is terminal. Flip-guard holds do not enter here.
///
/// # Errors
///
/// [`LifecycleRefusal::SCHEDULE_STALE_APPROVAL`] when the door's code is
/// one of the two wrapped codes — the runner raises, the door does not.
pub fn classify_door_refusal(
    refusal: DoorRefusal,
    attempt: i32,
    budget: AttemptBudget,
) -> Result<RunFinish, LifecycleRefusal> {
    if refusal.code == "STALE_REVISION" || refusal.code == "APPROVAL_REQUIRED" {
        return Err(LifecycleRefusal::schedule_stale_approval(format!(
            "door refused {}",
            refusal.code
        )));
    }
    if refusal.transient {
        if attempt + 1 >= budget.max {
            return Ok(RunFinish::Failed {
                reason: format!("transient budget exhausted after {attempt} attempts"),
            });
        }
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::TransientDependency,
            reason: format!("transient: {}", refusal.code),
        });
    }
    Ok(RunFinish::Failed {
        reason: refusal.code.to_owned(),
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
