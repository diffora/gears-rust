//! Legacy snapshot synthesis — the `migrated-origin` rule
//! (`inst-sy-freeze`, `inst-sy-select`, `inst-sy-payload`, `inst-sy-provenance`,
//! `inst-sy-firstrating`, `inst-sy-backdate`, D-76, D-81, D-87, D-102).
//!
//! A subscription with **no** `pricingSnapshotRef` cannot be rated: there is no
//! frozen economics to charge from. Synthesis builds one from published state as
//! of a per-trigger instant `t`, freezes it, and records how every part of it was
//! resolved. This module is the rule; [`crate::infra::synthesis`] is the reads.
//!
//! # The instant is per **trigger**, and the two are different facts
//!
//! D-81, restated design-side from the PRD's own acceptance criterion. For a
//! `migration` the instant is the **migration effective timestamp** — the moment
//! the subscriber's economics are about to change, so the state being frozen is
//! the state they are leaving. For `first-rating` it is the subscription's
//! **earliest unrated usage timestamp** — the moment the first charge would have
//! needed a price. Both UTC, both frozen at execution.
//!
//! Getting these the wrong way round is not a rounding error: it prices a
//! migrated subscriber at whatever the catalog said when someone happened to run
//! the job.
//!
//! # The two-tier lookup, and why tier 2 is unreachable today
//!
//! D-76, normative, per scope key of the subscription's frozen
//! `(currency, region)`:
//!
//! 1. **Live history first** — the `pricing_price` row, **current or
//!    superseded**, whose `PriceWindow` covered `t` on that key. The supersession
//!    chain is retained in-table, so this reproduces exactly what rating would
//!    have resolved at `t` and needs no import at all.
//! 2. **Reference set only if (1) is empty** — the `pricing_historical_price` row
//!    on that key with the greatest `effective_from <= t`, and `effective_to > t`
//!    where set. Its own interval substitutes for a window, because reference rows
//!    are never window-linked, and it may be **open-ended** (D-81) — which is what
//!    lets a still-in-effect legacy price cover a `migration`-trigger `t`.
//! 3. **Neither ⇒ fail closed.** Into the migration exception list, or the rating
//!    exception path for `first-rating`. Synthesis **never guesses a price and
//!    never falls back to the current row**, which is the clause the whole rule
//!    exists for: the current row is precisely the price the subscriber was *not*
//!    paying.
//!
//! **`pricing_historical_price` does not exist in the built system** — it is
//! Slice 5's `inst-bd-store`, and §1.7 records the absence normatively. So tier 2
//! always resolves empty and every key that misses tier 1 falls to clause (3).
//! That is the correct behaviour rather than a defect, and [`select_row`] is
//! written against the tier-2 *input* rather than around it: the day the store
//! lands, a non-empty `reference` changes no logic here.
//!
//! # The payload is self-contained because nothing can resolve its ids
//!
//! D-87, plus C-5's plan-level half. A `migrated-origin` ref resolves through
//! **no** `CatalogVersion` by construction — a tier-2 row exists in none, and a
//! tier-1 row's historical instant predates any useful pin — so Rating and
//! Billing cannot look anything up. Everything they need is materialized into the
//! frozen payload: the row content **and** the plan-level descriptor set and
//! resolved grant set, without which the payload is row-complete and
//! invoice-incomplete.
//!
//! # `first-rating` is never inline
//!
//! `inst-sy-firstrating`. When Rating meets a subscription with no snapshot the
//! line **fails closed into the rating exception path**; synthesis then runs as a
//! separate audited step and rating **retries** against the frozen result. It is
//! heavyweight, audited and grant-gated, and the rating hot path is none of those
//! things. [`SynthesisTrigger::runs_inline`] states it once so no caller has to
//! decide.

use std::fmt;

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// Why a snapshot is being synthesized (D-81, `inst-sy-freeze`).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SynthesisTrigger {
    /// A scheduled migration is about to move this subscriber. `t` is the
    /// **migration effective timestamp**.
    Migration,
    /// Rating met a subscription with no snapshot. `t` is the subscription's
    /// **earliest unrated usage timestamp**.
    FirstRating,
}

impl SynthesisTrigger {
    /// Every trigger, stable order.
    pub const ALL: &'static [Self] = &[Self::Migration, Self::FirstRating];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Migration => "migration",
            Self::FirstRating => "first_rating",
        }
    }

    /// Read a stored token back.
    ///
    /// `None` rather than a default, for [`crate::domain::migration::MigrationState::parse`]'s
    /// reason: the column's `CHECK` admits exactly two, so a third is a corrupt
    /// row and resolving it to either would attribute a snapshot to an instant
    /// rule it was not frozen under.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.as_str() == token)
    }

    /// May synthesis run **inline**, on the path that discovered the need?
    ///
    /// `false` for `first-rating` and that is `inst-sy-firstrating` entire: the
    /// rating line fails closed into the exception path, synthesis runs as a
    /// separate audited step, and rating retries against the frozen result.
    /// Synthesis is heavyweight, audited and grant-gated; the rating hot path is
    /// none of those.
    ///
    /// `true` for `migration` — scheduling is already an audited operator act
    /// with a transaction of its own, so there is no second path to defer to.
    #[must_use]
    pub const fn runs_inline(self) -> bool {
        matches!(self, Self::Migration)
    }
}

impl fmt::Display for SynthesisTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which tier of D-76's lookup a resolved row came from (`inst-sy-provenance`).
///
/// Recorded **per resolved id**, not per snapshot: one subscription's keys can
/// resolve from different tiers, and an auditor reconstructing a disputed charge
/// must be able to tell a real published price from a governed backdated
/// reconstruction without re-running the lookup.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionTier {
    /// Tier 1 — a `pricing_price` row, current or superseded, whose window
    /// covered `t`. This is what rating would actually have resolved.
    LiveHistory,
    /// Tier 2 — a `pricing_historical_price` reference row whose own interval
    /// covered `t`. Governed history, imported through Slice 5's backdating path.
    HistoricalImport,
}

impl SelectionTier {
    /// Every tier, stable order.
    pub const ALL: &'static [Self] = &[Self::LiveHistory, Self::HistoricalImport];

    /// The persisted / wire token (D-76's own spelling).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LiveHistory => "live_history",
            Self::HistoricalImport => "historical_import",
        }
    }

    /// Read a stored token back.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.as_str() == token)
    }
}

impl fmt::Display for SelectionTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A candidate row from tier 1 — a live or superseded price row whose window
/// covered `t`.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveCandidate {
    /// The `pricing_price` row.
    pub price_id: Uuid,
    /// The revision of the plan the row belonged to, where one is resolvable.
    pub plan_revision: Option<u64>,
}

/// A candidate row from tier 2 — a reference row whose own interval covered `t`.
///
/// **Nothing produces one of these today**: `pricing_historical_price` is Slice
/// 5's `inst-bd-store` and is unbuilt. The type exists so [`select_row`] is
/// written against the input rather than around its absence.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceCandidate {
    /// The `pricing_historical_price` row.
    pub historical_price_id: Uuid,
    /// When the reference interval opened. The selection rule takes the
    /// **greatest** `effective_from <= t`, so this is what orders candidates.
    pub effective_from: DateTime<Utc>,
}

/// One row selected for one scope key, and the tier it came from.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedRow {
    /// The resolved row's id — a `pricing_price` id on tier 1, a
    /// `pricing_historical_price` id on tier 2.
    pub row_id: Uuid,
    /// Which tier resolved it.
    pub tier: SelectionTier,
    /// The plan revision behind it, where there is one. `None` on tier 2, whose
    /// rows exist in no `CatalogVersion` by construction (D-87).
    pub plan_revision: Option<u64>,
}

/// Resolve one scope key against D-76's two tiers.
///
/// The caller supplies the candidates each tier produced **for that key at `t`**;
/// deciding which rows those are is a query and lives in
/// [`crate::infra::synthesis`]. What lives here is the part that must never
/// drift: the order the tiers are consulted in, and the refusal when neither
/// answers.
///
/// # Why tier 1 short-circuits
///
/// "Reference set **only if** (1) is empty" is D-76's own wording, and the reason
/// is that the two tiers are not equally good evidence: tier 1 *is* what rating
/// resolved at `t`, while tier 2 is a governed reconstruction of it. Consulting
/// both and preferring the newer, or merging them, would let an imported row
/// silently restate a price the system actually charged.
///
/// # Why more than one tier-2 candidate is not an error
///
/// The store's own uniqueness forbids overlapping reference intervals on a key
/// (`inst-bd-pipeline`), so at most one can cover `t` — but the selection rule is
/// stated as "the greatest `effective_from <= t`" and is implemented as stated
/// rather than as an assertion about the store. A rule that trusted the
/// uniqueness would be a second place for that invariant to live.
#[must_use]
pub fn select_row(live: &[LiveCandidate], reference: &[ReferenceCandidate]) -> Option<SelectedRow> {
    // (1) Live history first, and it short-circuits.
    if let Some(candidate) = live.first() {
        return Some(SelectedRow {
            row_id: candidate.price_id,
            tier: SelectionTier::LiveHistory,
            plan_revision: candidate.plan_revision,
        });
    }
    // (2) The reference set, greatest `effective_from` first.
    reference
        .iter()
        .max_by_key(|candidate| candidate.effective_from)
        .map(|candidate| SelectedRow {
            row_id: candidate.historical_price_id,
            tier: SelectionTier::HistoricalImport,
            // A reference row belongs to no revision. This is not "unknown": it
            // is the fact D-87 makes the payload self-contained *because* of.
            plan_revision: None,
        })
    // (3) `None` is fail-closed. The caller turns it into the exception list.
}

/// A scope key synthesis could not resolve a row for (`inst-sy-select` clause 3).
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedKey {
    /// The currency axis of the key.
    pub currency: String,
    /// The region axis.
    pub region: String,
}

/// What synthesis resolved for one subscription, before it is frozen.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthesisOutcome {
    /// The rows selected, one per resolved scope key.
    pub selected: Vec<SelectedRow>,
    /// The keys neither tier could answer for. **Non-empty means fail closed.**
    pub unresolved: Vec<UnresolvedKey>,
}

impl SynthesisOutcome {
    /// Refuse to freeze a snapshot that does not cover every key
    /// (`inst-sy-select` clause 3).
    ///
    /// **Partial synthesis is the one outcome that must not exist.** A snapshot
    /// missing a key is not a smaller snapshot: it is a subscription that will
    /// fail to rate on that key at some future instant, with a frozen record
    /// asserting that its economics were captured. The subscription goes to the
    /// exception list intact instead, where a human decides.
    ///
    /// A resolution set that is **empty** is refused by the same rule, and by the
    /// table's `chk_pricing_snapshot_provenance_resolved` underneath it.
    ///
    /// # Errors
    /// [`DomainError::ValidationFailed`] is deliberately **not** used — this is
    /// not a caller mistake. [`DomainError::PriceRowAbsent`] naming the keys,
    /// because the caller's next act is to import the missing history through
    /// Slice 5's backdating path or to scope the subscription out.
    pub fn ensure_complete(&self, subscription_ref: Uuid) -> Result<(), DomainError> {
        if self.unresolved.is_empty() && !self.selected.is_empty() {
            return Ok(());
        }
        let keys = self
            .unresolved
            .iter()
            .map(|key| format!("({}, {})", key.currency, key.region))
            .collect::<Vec<_>>()
            .join(", ");
        Err(DomainError::PriceRowAbsent(format!(
            "no published or reference price covers subscription {subscription_ref} on {} scope \
             key(s) at the synthesis instant: {keys}. Synthesis fails closed rather than pricing \
             the subscriber at the current row, which is precisely the price they were not paying",
            self.unresolved.len()
        )))
    }
}

#[cfg(test)]
#[path = "synthesis_tests.rs"]
mod synthesis_tests;
