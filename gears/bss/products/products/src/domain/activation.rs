//! The activation runner's claim protocol and failure posture
//! (`dod-activation-runner`, `dod-runner-failure-posture`,
//! `dod-scheduled-publish-pin`).
//!
//! # No privileged path
//!
//! The runner drives foundation doors. The publish call is
//! `GateMode::PreAuthorized(approval_id)` taken from the row's
//! `approval_ref` — P-D-105. Making the shipped host accept the
//! scheduled-flip pin is strand B's. This module is the caller.
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

use crate::domain::concurrency::InternalRevision;
use crate::domain::governance::{ApprovalId, GateMode, GateSubject, GateVerdict, GovernanceGate};

use super::lifecycle::LifecycleRefusal;
use super::retirement::RetirementHeld;

/// The reserved idempotency lane the runner resolves (`internal:` prefix).
pub const ACTIVATION_LANE: &str = "internal:scheduled-activation";

/// Written when the runner activates a cascade leg (**P-D-113** arm 6).
pub const CASCADE_LEG_LANE: &str = "internal:cascade-leg";

/// A pin mismatch is terminal. See [`verify_activation_pin`].
const PIN_MISMATCH_REASON: &str =
    "scheduled pin does not verify: the named record is not consumed or the row does not name it";

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

/// The cascade-leg key: the reserved lane plus the transition id as `client_key`.
#[must_use]
pub fn cascade_leg_idempotency_key(transition_id: Uuid) -> String {
    format!("{CASCADE_LEG_LANE}:{transition_id}")
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

/// The stored response body on either `internal:` lane (**P-D-123** item 7).
/// Defined once: lane + transition id is the key; this is the body.
#[must_use]
pub fn internal_lane_body(transition_id: Uuid, finish: &RunFinish) -> serde_json::Value {
    serde_json::json!({
        "transitionId": transition_id,
        "state": finish.state().as_str(),
        "outcomeReason": match finish {
            RunFinish::Applied => None,
            RunFinish::Failed { reason } | RunFinish::Deferred { reason, .. } => {
                Some(reason.as_str())
            }
        },
    })
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

/// The exact gate call the runner makes. The id is taken from the row
/// being flipped, never from a caller argument — that distinction is
/// P-D-105's safety.
///
/// Strand B owns host acceptance in `domain::approval`. This module
/// calls `evaluate` and admits only on [`GateVerdict::Authorized`].
/// [`scheduled_pin_holds`] stays as the domain statement of P-D-105;
/// it is not the admission decision. B's scheduled-flip arm is the
/// one that can answer `Authorized` for a cascade leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreAuthorizedCall {
    /// The approval pinned on the `ScheduledTransition` at scheduling.
    pub approval_id: Uuid,
}

impl PreAuthorizedCall {
    /// Build from the row's stored `approval_ref`. Never from a request.
    #[must_use]
    pub const fn from_row(approval_ref: Uuid) -> Self {
        Self {
            approval_id: approval_ref,
        }
    }

    /// `GateMode::PreAuthorized(approval_id)` — verify, do not consume.
    #[must_use]
    pub const fn mode_debug() -> &'static str {
        "GateMode::PreAuthorized(approval_id)"
    }

    /// The mode the runner passes to [`GovernanceGate::evaluate`].
    #[must_use]
    pub const fn mode(self) -> GateMode {
        GateMode::PreAuthorized(ApprovalId::new(self.approval_id))
    }
}

/// The pin a scheduled flip verifies. Both ids come from storage the
/// caller cannot write: the row's `approval_ref` and the named record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledActivation {
    /// `products_scheduled_transition.approval_ref` on the row being flipped.
    pub row_approval_ref: Uuid,
    /// The record the row names.
    pub record_id: Uuid,
    /// Whether that record is `consumed`.
    pub record_consumed: bool,
}

/// P-D-105: the named record is `consumed`, and the row names it in
/// `approval_ref`. Subject/revision equality is not asked here.
#[must_use]
pub fn scheduled_pin_holds(pin: &ScheduledActivation) -> bool {
    pin.record_consumed && pin.row_approval_ref == pin.record_id
}

/// Call the gate in `PreAuthorized` with the row's pin. Admission is the
/// host's [`GateVerdict::Authorized`] only; [`scheduled_pin_holds`] is the
/// domain statement of P-D-105 and is checked after a yes, never instead
/// of one.
///
/// A host [`GateVerdict::Refused`] finishes [`RunFinish::Deferred`] on
/// the transient-dependency arm. B's scheduled-flip arm landed in
/// `24e7d15f2`; the refusal is still transient because every door holds
/// [`crate::domain::governance::NoMaterialityPolicyGate`], which cannot
/// verify a pin — the missing piece is the host *wiring*, not the
/// predicate. After that wiring, a refusal that persists spends the
/// budget and fails. A pin mismatch after `Authorized` stays
/// [`RunFinish::Failed`]: a consumed-record mismatch will not become
/// true on the next poll.
pub fn verify_activation_pin(
    gate: &dyn GovernanceGate,
    subject: GateSubject,
    expected_revision: InternalRevision,
    pin: &ScheduledActivation,
) -> RunFinish {
    let call = PreAuthorizedCall::from_row(pin.row_approval_ref);
    match gate.evaluate(subject, expected_revision, call.mode()) {
        Ok(GateVerdict::Authorized(_)) => {}
        Ok(GateVerdict::Refused { reason }) => {
            return RunFinish::Deferred {
                population: DeferralPopulation::TransientDependency,
                reason: format!("activation gate refused: {reason}"),
            };
        }
        Err(error) => {
            return RunFinish::Failed {
                reason: format!("activation gate host failed: {error}"),
            };
        }
    }
    if scheduled_pin_holds(pin) {
        return RunFinish::Applied;
    }
    RunFinish::Failed {
        reason: PIN_MISMATCH_REASON.to_owned(),
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod activation_tests;
