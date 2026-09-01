//! The stored state machines, typed — one enum per `CHECK`-pinned token
//! column, mirroring the SDK's `LifecycleState` shape (`as_str`/`parse`)
//! so a comparison or a transition is checked by the compiler instead of
//! resting on dozens of scattered string literals.
//!
//! # The conversion happens at the storage boundary
//!
//! The repository parses a stored token into its enum when it builds a
//! record and renders `as_str()` when it writes one. `parse` answers
//! `None` for anything outside the roster rather than defaulting — the
//! fail-closed posture the gear takes everywhere: an unrecognised state is
//! a corrupt row (the migration's `CHECK` should have refused it), never a
//! default. The wire keeps its strings; a view renders `as_str()`.
//!
//! The demand lane already has its typed form — the SDK's
//! [`bss_products_sdk::increments::IncrementLane`] — so no twin is
//! declared here.

/// `products_catalog_version.freeze_state` — the ledger's derived cache
/// (`design/06`, P-D-67, P-D-84).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeState {
    /// At least one snapshotted participant has not confirmed.
    Open,
    /// Every participant confirmed (or the snapshot was empty).
    Complete,
    /// An operator forced the freeze past silent participants.
    CompleteForced,
}

impl FreezeState {
    /// The stable wire spelling — the `CHECK`-admitted column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Complete => "complete",
            Self::CompleteForced => "complete(forced)",
        }
    }

    /// Parse a stored value; `None` is a corrupt row, never a default.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "complete" => Some(Self::Complete),
            "complete(forced)" => Some(Self::CompleteForced),
            _ => None,
        }
    }
}

/// `products_freeze_ack.state` — one participant's row in the freeze
/// ledger (P-D-67, P-D-84).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeAckState {
    /// Seeded at version insert; the participant has not answered.
    Pending,
    /// The participant confirmed.
    Acked,
    /// The participant released without confirming.
    Released,
    /// An operator's force marked the row past its silence.
    NotFrozenForced,
}

impl FreezeAckState {
    /// The stable wire spelling — the `CHECK`-admitted column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Acked => "acked",
            Self::Released => "released",
            Self::NotFrozenForced => "not_frozen(forced)",
        }
    }

    /// Parse a stored value; `None` is a corrupt row, never a default.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "acked" => Some(Self::Acked),
            "released" => Some(Self::Released),
            "not_frozen(forced)" => Some(Self::NotFrozenForced),
            _ => None,
        }
    }
}

/// `products_catalog_version_request.state` — the increment queue's two
/// states (`design/06` §1.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    /// Enqueued, not yet drained.
    Pending,
    /// Satisfied by a committed catalog version.
    Coalesced,
}

impl RequestState {
    /// The stable wire spelling — the `CHECK`-admitted column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Coalesced => "coalesced",
        }
    }

    /// Parse a stored value; `None` is a corrupt row, never a default.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "coalesced" => Some(Self::Coalesced),
            _ => None,
        }
    }
}

/// `products_bulk_batch.state` — P-D-54's six plus P-D-69's `abandoned`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchState {
    /// Rows are being staged as drafts.
    Staging,
    /// Every row staged or failed; awaiting the approval.
    Reported,
    /// Approved for the commit phase.
    Approved,
    /// The commit phase is walking the rows.
    Committing,
    /// Terminal: committed.
    Completed,
    /// Terminal: failed.
    Failed,
    /// Terminal: abandoned (P-D-69).
    Abandoned,
}

impl BatchState {
    /// The three terminal states — the concurrent-batch ceiling counts
    /// everything outside them.
    pub const TERMINAL: [Self; 3] = [Self::Completed, Self::Failed, Self::Abandoned];

    /// The stable wire spelling — the `CHECK`-admitted column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Reported => "reported",
            Self::Approved => "approved",
            Self::Committing => "committing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }

    /// Parse a stored value; `None` is a corrupt row, never a default.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "staging" => Some(Self::Staging),
            "reported" => Some(Self::Reported),
            "approved" => Some(Self::Approved),
            "committing" => Some(Self::Committing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }
}

/// `products_reference_producer.state` — the registry's two states
/// (`design/07`, P-D-87).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerState {
    /// In the tenant's registered set; the predicate quantifies over it.
    Registered,
    /// Retired; its watermark and members were cleared with the move.
    Retired,
}

impl ProducerState {
    /// The stable wire spelling — the `CHECK`-admitted column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Retired => "retired",
        }
    }

    /// Parse a stored value; `None` is a corrupt row, never a default.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "registered" => Some(Self::Registered),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "states_tests.rs"]
mod states_tests;
