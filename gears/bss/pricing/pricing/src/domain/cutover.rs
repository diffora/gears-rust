//! The grandfathering cutover's compose-time judgement (`inst-gc-*`, `inst-co-*`).
//!
//! The cutover is the **second** sanctioned producer of `published → superseded`
//! (D-100), and D-127 binds it to the same unit guard as the interactive
//! supersession. Where the two units agree, this module calls the supersession's
//! spelling rather than restating it; where the design set gives them different
//! answers, it says which and why.

use chrono::{DateTime, Utc};

use crate::domain::error::DomainError;
use crate::domain::supersession::{ChangeoverMoment, changeover_floor};

/// Rule code for a cutover instant that has passed, or that no longer clears the
/// batching delay at approval commit (`07-pricewindow-linkage.md` §5, 422).
pub const CUTOVER_INSTANT_PASSED: &str = "CUTOVER_INSTANT_PASSED";

/// Is `cutover` far enough ahead of `now` for `moment`?
///
/// **One floor, two codes, and the design set is what settles it.** §5 declares
/// `SUPERSESSION_INSTANT_PASSED` as *"the same floor `inst-gc-compose` gives
/// cutovers, applied to the everyday mechanism"* — so the bound is
/// [`changeover_floor`], shared, and a second copy of it here would be the
/// hand-maintained duplicate that is how two mechanisms come to disagree about one
/// SLO. What is **not** shared is the code: an operator reading a refusal is told
/// which act they were performing, and §5 declares one code per unit.
///
/// The two floors are `inst-gc-compose`'s: strictly future at submit, and at least
/// the max batching-delay SLO ahead at approval commit. An instant inside that lag
/// would activate the successor's window while its row is not yet addressable at
/// any completed `CatalogVersion`, transiently failing renewals and arrears on the
/// key the cutover just closed.
///
/// # Errors
///
/// [`DomainError::CutoverInstantPassed`] naming the instant, the floor it missed,
/// which moment asked, and the remedy for that moment.
pub fn check_cutover_instant(
    cutover: DateTime<Utc>,
    now: DateTime<Utc>,
    moment: ChangeoverMoment,
) -> Result<(), DomainError> {
    changeover_floor(cutover, now, moment, "cutover").map_err(DomainError::CutoverInstantPassed)
}

#[cfg(test)]
#[path = "cutover_tests.rs"]
mod cutover_tests;
