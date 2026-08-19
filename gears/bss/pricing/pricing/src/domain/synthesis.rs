//! Legacy snapshot synthesis — the `migrated-origin` rule
//! (`inst-sy-freeze`, `inst-sy-select`, `inst-sy-payload`, `inst-sy-provenance`,
//! `inst-sy-firstrating`, D-76 as narrowed by D-330, D-81, D-87, D-102).
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
//! # The lookup, narrowed to one tier
//!
//! D-76 as narrowed by **D-330**, normative, per scope key of the subscription's
//! frozen `(currency, region)`:
//!
//! - **Clause (1), live history** — the `pricing_price` row, **current or
//!   superseded**, whose `PriceWindow` covered `t` on that key. The supersession
//!   chain is retained in-table, so this reproduces exactly what rating would have
//!   resolved at `t`.
//! - **Clause (3), otherwise ⇒ fail closed** — into the migration exception list,
//!   or the rating exception path for `first-rating`. Synthesis **never guesses a
//!   price and never falls back to the current row**, which is the clause the whole
//!   rule exists for: the current row is precisely the price the subscriber was
//!   *not* paying.
//!
//! **Clause (2) is struck, and the two survivors keep their numbers** rather than
//! closing the gap, because every citation of D-76 in this set and in the PRD names
//! them by those numbers. It read *"reference set only if (1) is empty — the
//! `pricing_historical_price` row on that key with the greatest `effective_from <=
//! t`"*, and the store behind it was Slice 5's `inst-bd-store`. **D-330
//! (2026-08-16) struck the whole historical-import flow**, `inst-bd-store` and
//! `inst-sy-backdate` — the instruction naming synthesis as its sanctioned consumer
//! — with it. This module had been carrying tier 2 as a *seam*: an empty candidate
//! slice passed into the rule on every synthesis, on the argument that "the day the
//! store lands, a non-empty `reference` changes no logic here". No such day is
//! coming, so the parameter is gone rather than left as a promise nothing intends to
//! keep. What is kept is the discriminator the tier was recorded under
//! ([`SelectionTier`]), because S11 §4 clause 2 keeps it: one value still tells a
//! reader which rule resolved the row.
//!
//! # The payload is self-contained because nothing can resolve its ids
//!
//! D-87, plus C-5's plan-level half. A `migrated-origin` ref resolves through
//! **no** `CatalogVersion` by construction — the resolved row's historical instant
//! predates any useful pin (C-5's second leg, a fully-legacy tier-2 key that
//! exists in no version at all, is struck with tier 2 by D-330 and the first leg
//! carries the rule alone) — so Rating and Billing cannot look anything up.
//! Everything they need is materialized into the frozen payload: the row content
//! **and** the plan-level descriptor set and resolved grant set, without which the
//! payload is row-complete and invoice-incomplete.
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

/// Which rule of D-76's lookup a resolved row came from (`inst-sy-provenance`).
///
/// Recorded **per resolved id**, not per snapshot: the discriminator belongs to
/// the row it describes, and an auditor reconstructing a disputed charge must be
/// able to see which rule resolved it without re-running the lookup.
///
/// **One variant, and the enum is kept rather than collapsed.** D-76 declared two
/// tiers; D-330 struck the second with the historical-import flow that was to fill
/// it, and S11 §4 clause 2 keeps the field anyway — *"a stored discriminator with
/// one value still tells a reader which rule resolved the row and is the seam any
/// later tier would land on"*. Collapsing the type would put the literal
/// `"live_history"` at both of `infra::synthesis`'s render sites and take the
/// stored token's one home away, which is the duplicated-conversion shape this
/// crate has been burned by; and `source` is provenance on a record with a
/// seven-year horizon, so dropping the wire field is a break with no benefit.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionTier {
    /// Tier 1 — a `pricing_price` row, current or superseded, whose window
    /// covered `t`. This is what rating would actually have resolved.
    LiveHistory,
}

impl SelectionTier {
    /// Every tier, stable order.
    pub const ALL: &'static [Self] = &[Self::LiveHistory];

    /// The persisted / wire token (D-76's own spelling).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LiveHistory => "live_history",
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

// `ReferenceCandidate` — a `pricing_historical_price` row whose own interval
// covered `t`, ordered by the greatest `effective_from <= t` — stood here and is
// **struck** (D-330, 2026-08-16). It was the input shape of D-76's tier 2, and its
// own doc said "nothing produces one of these today"; `inst-bd-store`, the store
// that was to, has left the design set with the whole historical-import flow, and
// `inst-sy-backdate` — the instruction naming synthesis as that flow's sanctioned
// consumer — went with it.

/// One row selected for one scope key, and the rule it came from.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedRow {
    /// The resolved `pricing_price` row's id.
    pub row_id: Uuid,
    /// Which rule resolved it. One value since D-330; see [`SelectionTier`].
    pub tier: SelectionTier,
    /// The plan revision behind it, where there is one.
    ///
    /// **`None` is a fact rather than a gap, and it is now tier 1's own fact.**
    /// It read "`None` on tier 2, whose rows exist in no `CatalogVersion` by
    /// construction (D-87)" — the tier that is struck. What makes the field an
    /// `Option` today is that `infra::synthesis::select_for_key` builds every
    /// candidate from a `PriceWindow`, which carries no revision, so a live
    /// resolution answers `None` too. D-87 obliges the self-contained payload
    /// either way; the difference is only which sentence says why.
    pub plan_revision: Option<u64>,
}

/// Resolve one scope key against D-76's live-history rule.
///
/// The caller supplies the candidates the rule produced **for that key at `t`**;
/// deciding which rows those are is a query and lives in
/// [`crate::infra::synthesis`]. What lives here is the part that must never
/// drift: the refusal when nothing answers.
///
/// # Why there is one rule and not two
///
/// D-76 declared two tiers and this function consulted them in order, tier 1
/// short-circuiting tier 2 because *"the two tiers are not equally good evidence:
/// tier 1 **is** what rating resolved at `t`, while tier 2 is a governed
/// reconstruction of it"*. **D-330 struck tier 2** with the historical-import flow
/// that was to populate it, so the order is no longer a rule this function can
/// get wrong. The judgement that produced the order survives its subject: a
/// reconstruction must never silently restate a price the system actually
/// charged, which is why nothing may be merged into the live set here and why
/// clause (3) fails closed instead of reaching for the current row.
///
/// # Why it answers with a **set**
///
/// A [`FrozenKey`] is D-76's frozen `(currency, region)` pair — a **market**, not
/// a canonical scope key — and a market legitimately carries more than one live
/// line: `inst-cs-hybrid` exists to sanction a recurring row beside a usage row,
/// and an `existing_grandfathered` generation sits beside an `all_subscriptions`
/// row on the same pair. This returned `Option` until 2026-08-11 and the caller
/// kept the lowest `price_id`, so every other line on the market was dropped in
/// silence.
///
/// That is not a smaller snapshot. D-87 makes the frozen payload self-contained —
/// Rating evaluates from it and Billing posts from it, resolving nothing through
/// the read model — so **a dropped line is a charge that never happens**, on a
/// record with a seven-year horizon and no way to notice.
#[must_use]
pub fn select_rows(live: &[LiveCandidate]) -> Vec<SelectedRow> {
    // (1) Live history — as a set.
    //
    // (3) An empty set is fail-closed, and it is what an empty input becomes here:
    //     the caller turns it into the exception list, exactly as it did when this
    //     answered `None` and when clause (2) still stood between the two.
    live.iter()
        .map(|candidate| SelectedRow {
            row_id: candidate.price_id,
            tier: SelectionTier::LiveHistory,
            plan_revision: candidate.plan_revision,
        })
        .collect()
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
