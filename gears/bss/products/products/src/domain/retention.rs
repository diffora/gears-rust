//! The retention gate — whether a `CatalogVersion`'s manifest rows may be
//! collected (`design/10-retention-erasure.md`; **P-D-49**,
//! `dod-retention-gate`).
//!
//! # The domain is the version's own snapshot, never the registration rows
//!
//! **P-D-49**: the gate ranges over that version's `participant_set_snapshot`
//! and **not** over whatever registration rows exist. Both halves of that
//! sentence are corrections with a named failure behind them, and the `DoD`
//! states them:
//!
//! - Quantifying over **registrations** let an **empty ledger** satisfy the
//!   gate *vacuously* and collect a version nobody had frozen. So a snapshot
//!   member with **no registration row holds** the version — the freeze
//!   fan-out has not reached it yet, and absence is not release.
//! - An **empty snapshot** is collectable, because nobody ever owed an ack.
//!   That is not the same vacuity: the domain is empty by the version's own
//!   record rather than by a store the fan-out has not filled.
//!
//! # The two arms are a pair, and the timestamp alone is not one of them
//!
//! Every registration must satisfy `state = released`, **or**
//! `state = not_frozen(forced)` **and** `released_at` stamped. Reading the
//! **timestamp alone** collected a version holding live grandfathered
//! references, because nothing clears the stamp: a forced participant that
//! later recovered and acked leaves `state = acked` beside a live
//! `released_at` (P-D-67 — *"the state moving is what makes the stamp
//! inert"*). And reading the **state alone** would be wrong in the other
//! direction, because a door-released row carries `state = released` with the
//! stamp **NULL** while a forced row carries both — which is why
//! [`FreezeRegistration`] carries them separately rather than deriving one
//! from the other.
//!
//! # A held version is skipped, never forced
//!
//! C4: a candidate with a live registration is **skipped** with
//! `retention_orphan_blocked` — fail closed. This module answers the
//! predicate and names the reason; nothing here deletes, and no caller may
//! read a `Hold` as a soft warning.
//!
//! # What this module deliberately does not decide
//!
//! Its operand is `06-catalog-version`'s `inst-fz-liveness`, and that
//! feature's §7 rows 6, 11 and 33 hold the open half — whether
//! `freezeComplete`'s formula is restated to match this predicate, the
//! ledger's transition table, and who writes `released_at`. **Cited, not
//! re-raised**: this predicate reads the ledger as it ships and takes no
//! position on those three.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-retention-gate:p1

use crate::infra::storage::repo::FreezeRegistration;

use crate::domain::states::FreezeAckState;

/// Why a version's manifest rows are held back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetentionHold {
    /// A snapshot member has no registration row at all — the fan-out has not
    /// reached it, and absence is never release.
    NoRegistration {
        /// The participant the snapshot names.
        participant: String,
    },
    /// A registration exists and is live: neither released nor
    /// forced-with-a-stamp.
    LiveRegistration {
        /// The participant.
        participant: String,
        /// Its state, for the skip reason.
        state: FreezeAckState,
    },
    /// The forced arm without its stamp — the shape `CHECK` refuses this on
    /// both engines, so reaching it means a row was written past the guard.
    ForcedWithoutStamp {
        /// The participant.
        participant: String,
    },
}

impl RetentionHold {
    /// The participant this hold is about.
    #[must_use]
    pub fn participant(&self) -> &str {
        match self {
            Self::NoRegistration { participant }
            | Self::LiveRegistration { participant, .. }
            | Self::ForcedWithoutStamp { participant } => participant,
        }
    }

    /// The skip reason C4 names — **one constant for every arm**, because the
    /// requirement is that a held candidate is skipped and never forced,
    /// whatever holds it. As a const rather than a method, "every arm carries
    /// the same reason" is true by construction and needs no probe; the test
    /// that asserted it was a tautology and is deleted rather than kept.
    pub const REASON: &'static str = "retention_orphan_blocked";
}

/// The gate's verdict for one `CatalogVersion`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetentionVerdict {
    /// Every snapshot member is released, or the snapshot is empty.
    Collectable,
    /// At least one member holds the version. **Every** hold is reported, not
    /// the first: an operator repairing one and re-running would otherwise
    /// discover the rest one pass at a time.
    Held(Vec<RetentionHold>),
}

/// Evaluate the gate over one version's snapshot and its registrations.
///
/// `snapshot` is the participant set the version froze — the members of its
/// own `participant_set_snapshot`, already parsed. `registrations` is the
/// ledger as it stands.
#[must_use]
pub fn evaluate(snapshot: &[String], registrations: &[FreezeRegistration]) -> RetentionVerdict {
    let mut holds = Vec::new();
    for participant in snapshot {
        let Some(row) = registrations
            .iter()
            .find(|row| row.participant == *participant)
        else {
            holds.push(RetentionHold::NoRegistration {
                participant: participant.clone(),
            });
            continue;
        };
        match row.state {
            FreezeAckState::Released => {}
            FreezeAckState::NotFrozenForced if row.released_at_stamped => {}
            FreezeAckState::NotFrozenForced => holds.push(RetentionHold::ForcedWithoutStamp {
                participant: participant.clone(),
            }),
            state => holds.push(RetentionHold::LiveRegistration {
                participant: participant.clone(),
                state,
            }),
        }
    }
    if holds.is_empty() {
        RetentionVerdict::Collectable
    } else {
        RetentionVerdict::Held(holds)
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
