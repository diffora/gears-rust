//! The row lifecycle state machine.
//!
//! [`LifecycleState::transition`] is the **single source of truth** for what is
//! legal. The database trigger's column whitelist and the authoring surfaces
//! both restate the machine in their own vocabulary; if the legal edge set
//! lived in three places, the one that drifted would be discovered as a
//! published row in a state nothing can move it out of.
//!
//! Legal edges, and nothing else:
//!
//! ```text
//! draft      -> published
//! published  -> superseded
//! published  -> retired
//! ```
//!
//! `published -> superseded` has exactly **two** sanctioned producers: the
//! supersession unit, and the grandfathering cutover commit (D-100) whose
//! `all_subscriptions` successor lands on the predecessor's own key. Both set
//! `supersedes_price_id` on the successor, which is what gives the unit guard
//! its comparison referent (D-127). No third path may produce this flip.
//!
//! There is deliberately **no self-edge**: re-publishing a published row is not
//! a no-op, it is a request whose intent the machine cannot infer, and treating
//! it as one would let a retry silently pass through the same guard twice.

use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::error::DomainError;

/// The lifecycle state of a plan revision or price row.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LifecycleState {
    /// Authoring state. The only state whose content may still change.
    #[default]
    Draft,
    /// Live and consumer-visible: projected into the read model, pinnable,
    /// covered by the append-only discipline.
    Published,
    /// Replaced on its canonical scope key by a successor. The row is retained
    /// as history and its window may still be active until the changeover;
    /// what it is not is the **current** row on its key.
    Superseded,
    /// Ended. **Terminal**, and the terminality is load-bearing: a retired plan
    /// can never publish again, so nothing would ever re-project it. That is
    /// precisely why retirement had to become a publish unit of its own
    /// (D-128) — validation, a pending `CatalogVersion` ref, a plan-subject
    /// re-projection, a warm — rather than an in-place edit of a frozen
    /// version. Had it stayed an edit, the read model would have advertised a
    /// retired plan as sellable **permanently**, with no later publish able to
    /// correct it.
    Retired,
}

impl LifecycleState {
    /// Every state, stable order.
    pub const ALL: &'static [Self] = &[
        Self::Draft,
        Self::Published,
        Self::Superseded,
        Self::Retired,
    ];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Superseded => "superseded",
            Self::Retired => "retired",
        }
    }

    /// May this row's **content** still change?
    ///
    /// Only in `draft`. A published row is immutable except for the two
    /// whitelisted column moves the trigger allows — the state flips this
    /// machine sanctions, and monotonic tightening of `grandfather_until` —
    /// neither of which is content.
    #[must_use]
    pub const fn is_content_mutable(self) -> bool {
        matches!(self, Self::Draft)
    }

    /// Is this the **current** revision of its subject?
    ///
    /// `published` **or `retired`** (D-128). The wider set is not a convenience:
    /// the projector sources a plan subject from its current revision, and
    /// after retirement flips the only published one there would otherwise be
    /// no referent at all — a re-warm or a degraded re-drive would project an
    /// empty plan subject and break resolution for exactly the in-flight
    /// subscribers a retired plan must keep pricing. `superseded` is excluded
    /// because a successor has taken the key.
    #[must_use]
    pub const fn is_current_revision(self) -> bool {
        matches!(self, Self::Published | Self::Retired)
    }

    /// Is a move from `self` to `next` legal?
    #[must_use]
    pub const fn can_transition(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Published) | (Self::Published, Self::Superseded | Self::Retired)
        )
    }

    /// Assert a transition, refusing every edge the machine does not sanction.
    ///
    /// # Errors
    ///
    /// [`DomainError::LifecycleForbidden`] for any edge outside the three legal
    /// ones — including every self-edge, every move out of a terminal state,
    /// and every attempt to walk the machine backwards.
    pub fn transition(self, next: Self) -> Result<(), DomainError> {
        if self.can_transition(next) {
            return Ok(());
        }
        Err(DomainError::LifecycleForbidden(format!("{self} -> {next}")))
    }
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
