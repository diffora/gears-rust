//! The supersession unit's own domain rules — the ones that are preconditions
//! rather than pipeline rules.
//!
//! The unit's **content** rule lives elsewhere and is a
//! [`ValidationRule`](crate::domain::validation::ValidationRule):
//! [`SupersessionPair`](crate::domain::rules::SupersessionPair) is the D-82/D-98/
//! D-122/D-127/D-129 unit guard, and it belongs in the rule set because it judges
//! a pair of rows and reports violations alongside every other row rule. What is
//! here judges neither rows nor shape — it judges the **changeover instant**, and
//! it is asked twice about the same value at two different moments.
//!
//! # Why the floor is two floors (`inst-su-instant`)
//!
//! The instant MUST be strictly future **at submit** and at least the max
//! batching-delay SLO in the future **at approval commit**. Those are not two
//! spellings of one rule and neither implies the other in the direction that
//! matters:
//!
//! - Holding a **submit** to the commit floor would refuse a unit that is going to
//!   be perfectly legal by the time a second person approves it. A submit is a
//!   proposal; the batching delay has not started running against it.
//! - Holding a **commit** to the submit floor is the money defect the rule exists
//!   for. `CatalogVersion` assignment batches (D-47: p95 ≤ 60s, max 5 min), so an
//!   instant inside that lag activates the successor's window while its row is not
//!   yet addressable at any *completed* version — renewals and arrears fail closed
//!   for up to the whole delay, multiplied across every key of a repricing run.
//!
//! The design set gives the second refusal a remedy rather than a retry: the unit
//! is **recomposed** against a fresh instant. That word is in the message on
//! purpose — a caller told only "422" would resubmit the same instant, which is
//! refused identically and further behind the floor each time.
//!
//! # The delay is a constant here and a knob in `config`, deliberately
//!
//! [`MAX_BATCHING_DELAY`] is 5 minutes, D-47's ratified maximum, and it is **not**
//! read from [`JobsConfig`](crate::config::JobsConfig) even though
//! `catalog_version_overdue_secs` defaults to the same 300 seconds from the same
//! decision. The two are the same number with different natures: that field is an
//! **alarm threshold**, an ops question about how long to wait before shouting, and
//! lowering it costs an operator some noise. This is a **correctness floor**, and
//! lowering it buys a tenant transiently unrateable subscriptions. A floor that a
//! configuration file can move is not a floor.
//!
//! `inst-gc-compose` holds the cutover's changeover to the same bound. When that
//! unit is built it takes this constant, not a copy of the number — a
//! hand-maintained second copy is how the two mechanisms come to disagree about
//! the same SLO.

use chrono::{DateTime, Duration, Utc};
use toolkit_macros::domain_model;

use crate::domain::error::DomainError;

/// The wire code §5 declares for a changeover instant that has fallen behind its
/// floor (architectural 422, rendered 400 — Foundation §3.3).
pub const SUPERSESSION_INSTANT_PASSED: &str = "SUPERSESSION_INSTANT_PASSED";

/// D-47's ratified **maximum** `CatalogVersion` batching delay, and the distance
/// `inst-su-instant` requires a changeover to clear at approval commit.
///
/// The p95 is 60s and this is the max; the floor takes the max because the floor's
/// job is to be right for the slowest batch rather than the median one.
pub const MAX_BATCHING_DELAY: Duration = Duration::minutes(5);

/// Which of `inst-su-instant`'s two floors a changeover instant is being held to.
///
/// An enum rather than two functions, because the two questions differ **only** in
/// the bound and a caller choosing between two similarly-named functions is a
/// caller who can pick the lenient one at the strict moment. Here the moment is
/// the argument and the bound is derived from it.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeoverMoment {
    /// Submit: the instant must be strictly in the future.
    Submit,
    /// Approval commit: the instant must additionally clear the whole batching
    /// delay.
    Commit,
}

impl ChangeoverMoment {
    /// How far ahead of `now` this moment requires the instant to be.
    ///
    /// `Submit`'s bound is zero and the comparison below is strict, which is what
    /// makes "strictly future" and "at least one delay ahead" one expression
    /// instead of two branches that could drift apart.
    const fn margin(self) -> Duration {
        match self {
            Self::Submit => Duration::zero(),
            Self::Commit => MAX_BATCHING_DELAY,
        }
    }

    /// What the refusal tells the operator to do about it.
    const fn remedy(self) -> &'static str {
        match self {
            Self::Submit => "name a future changeover instant",
            Self::Commit => {
                "the unit must be recomposed against a changeover at least 300s (5 min) ahead"
            }
        }
    }
}

/// Is `changeover` far enough ahead of `now` for `moment`?
///
/// # Errors
/// [`DomainError::SupersessionInstantPassed`] naming the instant, the floor it
/// missed and the remedy for that moment.
pub fn check_changeover_instant(
    changeover: DateTime<Utc>,
    now: DateTime<Utc>,
    moment: ChangeoverMoment,
) -> Result<(), DomainError> {
    let floor = now + moment.margin();
    if changeover > floor {
        return Ok(());
    }
    Err(DomainError::SupersessionInstantPassed(format!(
        "changeover instant {} is not strictly after {} at {}; {}",
        changeover.to_rfc3339(),
        floor.to_rfc3339(),
        match moment {
            ChangeoverMoment::Submit => "submit",
            ChangeoverMoment::Commit => "approval commit",
        },
        moment.remedy()
    )))
}

#[cfg(test)]
#[path = "supersession_tests.rs"]
mod supersession_tests;
