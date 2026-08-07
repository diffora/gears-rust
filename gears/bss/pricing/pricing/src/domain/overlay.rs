//! The `PriceOverlay` vocabulary — scope, lines, magnitudes and the resolution
//! rule (`design/09-price-overlays.md` §6, D-08, D-42, D-67, D-78, D-138).
//!
//! Where [`crate::domain::overlay_rules`] judges a whole candidate against the
//! world, this module is the **shapes** — and several of the design set's rules
//! are enforced here by being unrepresentable rather than checked, which is
//! stronger than a `CHECK` and cheaper than a rule:
//!
//! * **`(scope_class = 'global') = (scope_value = '')`** — [`ScopeSelector`] is a
//!   closed enum whose `Global` arm carries no value and whose other five arms
//!   each carry one. There is no way to build a classless overlay with a value.
//! * **`fixed` is always amount-based (D-138)** — [`Adjustment::Fixed`] carries
//!   an [`AmountSet`] and no `Magnitude`, so a percentage `fixed` line does not
//!   typecheck. This is the one rule the store spends a `CHECK` on that the
//!   domain makes impossible.
//! * **A `targetSku` line names its plan, and a `cohort` line does too** —
//!   [`LineKey`] has four constructors and no public fields, so the two
//!   `CHECK`s that pair those columns with `plan_id` have no representable
//!   counterexample in this crate.
//! * **The magnitude's type is declared, never inferred (D-08)** —
//!   [`Magnitude`] is a sum, so "a percent value and an amount set at once" and
//!   "neither" are both unconstructible.
//!
//! # What is deliberately **not** enforced here
//!
//! **D-67's ranges.** `0 < v <= 10000` on a discount and `v > 0` on a markup are
//! [`overlay_rules`](crate::domain::overlay_rules)' checks and not constructor
//! refusals, and the reason is §5's own wording: the refusal is
//! `ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE` **naming the line**. A constructor cannot
//! name a line in an aggregate report, and the aggregate report is what makes an
//! overlay remediable in one pass rather than in as many round trips as it has
//! faults (Foundation §4.2). The store's two `CHECK`s are the backstop; the two
//! guards are in series, so the pure rule is unit-tested directly rather than
//! only through a path the store would have refused anyway.
//!
//! **Evaluation.** `markup` adds, `discount` subtracts and `fixed` replaces
//! (D-138), and none of that arithmetic is here: evaluation is Tariffs'. What
//! this crate owes is the authored shape, the resolution rule
//! [`resolve_line`] publishes, and the precedence and class order the read model
//! carries. In particular **no sort direction is encoded anywhere**, because
//! §F.1 leaves it undecided — see [`crate::domain::overlay_rules`] for the one
//! place that absence bites.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::{PlanId, PriceEligibility};

/// The six scope classes of §1.1, in the **class-specificity order** the read
/// model publishes as `inst-plv-class-tiebreak`'s tie-break:
/// `customerGroup > partner > orgTier > brand > region > global`.
///
/// Declared **least specific first**, so the derived [`Ord`] ranks them exactly
/// as that order does and no later code can build a second, disagreeing ranking
/// out of this type. That is [`PriceEligibility`]'s trick, applied to a
/// different order for the same reason.
///
/// **It is a rank, not a direction.** The class order breaks ties *inside* a
/// stack and never filters an overlay out — overlay application is stack-all,
/// never single-winner (`inst-plv-class-tiebreak`). The stack is walked
/// **ascending** since D-249 (2026-08-07) — lowest `precedence` first, most
/// specific last — which this type expresses exactly: the derived `Ord` ranks
/// least-specific lowest, so a class that sorts below another is applied before
/// it and is the one a `fixed` layer discards.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeClass {
    /// The classless overlay — every payer of the tenant.
    Global,
    /// A pricing region.
    Region,
    /// A commercial brand.
    Brand,
    /// An organisation tier (D-120).
    OrgTier,
    /// A reseller / partner (D-120).
    Partner,
    /// A BSS customer group — the most specific class.
    CustomerGroup,
}

impl ScopeClass {
    /// Every class, least specific first.
    pub const ALL: &'static [Self] = &[
        Self::Global,
        Self::Region,
        Self::Brand,
        Self::OrgTier,
        Self::Partner,
        Self::CustomerGroup,
    ];

    /// The persisted / wire token, `snake_case` as the column and the wire both
    /// are (`toolkit_macros::api_dto` does not rename).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Region => "region",
            Self::Brand => "brand",
            Self::OrgTier => "org_tier",
            Self::Partner => "partner",
            Self::CustomerGroup => "customer_group",
        }
    }

    /// Parse a stored or authored token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == token)
    }

    /// The physical taxonomy table this class's values are declared in, or
    /// `None` for [`ScopeClass::Global`], which has no value universe because it
    /// has no value.
    ///
    /// Named rather than returned as a repository handle so the domain stays
    /// free of infrastructure (DE0301): what the rule needs to *say* is which
    /// universe was consulted, and the consulting is
    /// [`overlay_repo`](crate::infra::storage::repo::overlay_repo)'s.
    #[must_use]
    pub const fn taxonomy_table(self) -> Option<&'static str> {
        match self {
            Self::Global => None,
            Self::Region => Some("pricing_region_taxonomy"),
            Self::Brand => Some("pricing_brand_taxonomy"),
            Self::OrgTier => Some("pricing_org_tier_taxonomy"),
            Self::Partner => Some("pricing_partner_taxonomy"),
            // The customer-group taxonomy belongs to the membership half, which
            // is not built. `ScopeSelector::customer_group` is refused at the
            // authoring edge for that reason; see its own doc.
            Self::CustomerGroup => Some("pricing_customer_group_taxonomy"),
        }
    }
}

impl fmt::Display for ScopeClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A declared taxonomy value. Never blank — the empty string is the store's
/// sentinel for the classless scope, so a blank value would make that sentinel
/// forgeable.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeValue(String);

impl ScopeValue {
    /// Wrap an authored value, refusing a blank one.
    #[must_use]
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    /// The value as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScopeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An overlay's scope: a class, and a value exactly when the class has one.
///
/// The store spends a biconditional `CHECK` on this pairing
/// (`chk_pricing_price_overlay_scope_value`); here it is unrepresentable, which
/// is why no rule in [`overlay_rules`](crate::domain::overlay_rules) checks it.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeSelector {
    /// Every payer of the tenant. Carries no value.
    Global,
    /// A class with a declared value.
    Scoped {
        /// The class — never [`ScopeClass::Global`], which the constructor
        /// refuses.
        class: ScopeClass,
        /// The value, validated against `class.taxonomy_table()` by
        /// `inst-plv-scope`.
        value: ScopeValue,
    },
}

impl ScopeSelector {
    /// Build a scoped selector, refusing [`ScopeClass::Global`] — which would be
    /// a classless overlay carrying a value, the state the biconditional
    /// forbids.
    #[must_use]
    pub fn scoped(class: ScopeClass, value: ScopeValue) -> Option<Self> {
        (class != ScopeClass::Global).then_some(Self::Scoped { class, value })
    }

    /// The selector's class.
    #[must_use]
    pub const fn class(&self) -> ScopeClass {
        match self {
            Self::Global => ScopeClass::Global,
            Self::Scoped { class, .. } => *class,
        }
    }

    /// The selector's value, or `None` for the classless scope.
    #[must_use]
    pub const fn value(&self) -> Option<&ScopeValue> {
        match self {
            Self::Global => None,
            Self::Scoped { value, .. } => Some(value),
        }
    }

    /// How the store renders the value — the empty string for the classless
    /// scope. See `m20260802_000032` for why the sentinel and not a NULL.
    #[must_use]
    pub fn stored_value(&self) -> &str {
        self.value().map_or("", ScopeValue::as_str)
    }
}

/// L5's declared tax basis. `NOT NULL` in the store, and a closed enum here —
/// which is exactly why *"silence fails"* needs an entry point of its own; see
/// [`overlay_rules::check_tax_basis_declared`](crate::domain::overlay_rules::check_tax_basis_declared).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaxBasis {
    /// The adjustment is stated tax-inclusive.
    Inclusive,
    /// The adjustment is stated tax-exclusive.
    Exclusive,
    /// Explicitly delegated to Tariffs — a **declaration**, not a silence.
    DelegatedTariffs,
}

impl TaxBasis {
    /// Every basis, in §6's order.
    pub const ALL: &'static [Self] = &[Self::Inclusive, Self::Exclusive, Self::DelegatedTariffs];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inclusive => "inclusive",
            Self::Exclusive => "exclusive",
            Self::DelegatedTariffs => "delegated_tariffs",
        }
    }

    /// Parse a stored or authored token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|b| b.as_str() == token)
    }
}

/// L6's consumer-facing exposure flag.
///
/// `restricted` means the overlay — **including its existence** — is never
/// exposed on a consumer-facing enumeration or preview and resolves only inside
/// the member payer's own evaluation context. Operator and service reads are
/// unaffected: the flag governs consumer-facing exposure only, and this gear
/// mounts no consumer-facing enumeration at all (see
/// [`overlay_rules`](crate::domain::overlay_rules) for what that leaves owed).
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Disclosure {
    /// The fail-closed default.
    #[default]
    Restricted,
    /// Presentation and the Tariffs effective-price preview may disclose the
    /// adjusted price to anyone.
    Public,
}

impl Disclosure {
    /// Both values, default first.
    pub const ALL: &'static [Self] = &[Self::Restricted, Self::Public];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Restricted => "restricted",
            Self::Public => "public",
        }
    }

    /// Parse a stored or authored token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|d| d.as_str() == token)
    }
}

/// The lifecycle of one overlay **revision** (D-92).
///
/// Three states and no `abandoned` tombstone, unlike `pricing_plan`: a
/// discarded draft revision leaves by DELETE, which is the only exit it has.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverlayLifecycle {
    /// The editable revision. At most one per overlay.
    #[default]
    Draft,
    /// The revision consumers resolve.
    Published,
    /// A predecessor, flipped by the submit that published its successor.
    Superseded,
    /// A discarded draft's tombstone (D-231). Terminal: it never publishes, never
    /// supersedes, and is never a candidate for a read.
    ///
    /// It exists to hold the **revision number** the discarded draft consumed. A
    /// discarded draft used to leave by `DELETE`, so `max(revision) + 1` re-minted
    /// its number and a client holding the discarded revision's entity tag found a
    /// different revision under the same overlay identity — its stale `If-Match`
    /// matching the fresh row's compare-and-swap. `pricing_plan` has carried the
    /// same tombstone since D-145, for the reason stated in the same words.
    Abandoned,
}

impl OverlayLifecycle {
    /// Every state, in §6's order.
    pub const ALL: &'static [Self] = &[
        Self::Draft,
        Self::Published,
        Self::Superseded,
        Self::Abandoned,
    ];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Superseded => "superseded",
            Self::Abandoned => "abandoned",
        }
    }

    /// Parse a stored token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.as_str() == token)
    }
}

/// A SKU reference on a line. Never blank, for [`ScopeValue`]'s reason: the
/// store's line key coalesces an absent SKU to the empty string.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetSku(String);

impl TargetSku {
    /// Wrap an authored SKU, refusing a blank one.
    #[must_use]
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    /// The SKU as stored.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetSku {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One line's key inside a revision: `(planId?, targetSku?, cohort?)`.
///
/// Four constructors and no public fields, because two of §6's `CHECK`s are
/// pairings this type can simply not violate: a `targetSku` requires a
/// `planId` (a bare SKU is ambiguous per `(currency, region)`) and a `cohort`
/// requires one too (`inst-plv-eligibility` validates a cohort against *"the
/// line's target plan"*, which the list-default line has none of).
///
/// **`cohort` is a filter and not a level**, which is why it is in the key and
/// **not** in [`LineKey::specificity`]: a cohort-targeted line and a cohort-less
/// line on one `(plan, sku)` are disjoint by eligibility rather than ranked
/// against each other (D-78).
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineKey {
    plan_id: Option<PlanId>,
    target_sku: Option<TargetSku>,
    cohort: Option<DateTime<Utc>>,
}

impl LineKey {
    /// The list-default line: no plan, no SKU, no cohort. It applies to every
    /// target of the overlay's `target_ref`.
    #[must_use]
    pub const fn list_default() -> Self {
        Self {
            plan_id: None,
            target_sku: None,
            cohort: None,
        }
    }

    /// A per-plan line.
    #[must_use]
    pub const fn for_plan(plan_id: PlanId) -> Self {
        Self {
            plan_id: Some(plan_id),
            target_sku: None,
            cohort: None,
        }
    }

    /// A per-`(plan, sku)` line — the most specific level.
    #[must_use]
    pub const fn for_sku(plan_id: PlanId, target_sku: TargetSku) -> Self {
        Self {
            plan_id: Some(plan_id),
            target_sku: Some(target_sku),
            cohort: None,
        }
    }

    /// Narrow this key to one grandfathered generation (D-78).
    ///
    /// Takes `self` rather than being a fifth constructor so the `plan_id`
    /// requirement is carried by the key it narrows: `list_default().for_cohort`
    /// is refused, which is §6's `CHECK (cohort IS NULL OR plan_id IS NOT NULL)`
    /// with no `CHECK`.
    #[must_use]
    pub fn for_cohort(self, cohort: DateTime<Utc>) -> Option<Self> {
        self.plan_id.is_some().then_some(Self {
            cohort: Some(cohort),
            ..self
        })
    }

    /// The line's target plan, or `None` on the list-default line.
    #[must_use]
    pub const fn plan_id(&self) -> Option<PlanId> {
        self.plan_id
    }

    /// The line's SKU narrowing.
    #[must_use]
    pub const fn target_sku(&self) -> Option<&TargetSku> {
        self.target_sku.as_ref()
    }

    /// The generation this line filters to, or `None` for the ordinary
    /// eligibility classes.
    #[must_use]
    pub const fn cohort(&self) -> Option<DateTime<Utc>> {
        self.cohort
    }

    /// The most-specific rank of `inst-plv-lines`: `(plan, sku)` > `(plan)` >
    /// default, as `2 > 1 > 0`.
    ///
    /// **`cohort` is deliberately not an input.** It selects which rows a line
    /// is *eligible* for; the ranking then runs unchanged inside the eligible
    /// set, so nothing about existing resolution moves (D-78).
    #[must_use]
    pub const fn specificity(&self) -> u8 {
        match (self.plan_id.is_some(), self.target_sku.is_some()) {
            (true, true) => 2,
            (true, false) => 1,
            // A SKU without a plan is unconstructible, so this arm is the
            // list-default line and nothing else.
            (false, _) => 0,
        }
    }

    /// Is this line eligible for a row of `eligibility` at `cohort`?
    ///
    /// D-78, and it is a **filter**: a `cohort`-less line applies only to
    /// `all_subscriptions` / `new_subscriptions_only` rows, and a
    /// `cohort`-carrying line applies **only** to `existing_grandfathered` rows
    /// of that generation.
    #[must_use]
    pub fn eligible_for(
        &self,
        eligibility: PriceEligibility,
        cohort: Option<DateTime<Utc>>,
    ) -> bool {
        match self.cohort {
            None => eligibility != PriceEligibility::ExistingGrandfathered,
            Some(generation) => {
                eligibility == PriceEligibility::ExistingGrandfathered && cohort == Some(generation)
            }
        }
    }
}

/// An amount-based magnitude: money, and therefore **per currency** (D-08, no
/// implicit FX).
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AmountSet(BTreeMap<CurrencyCode, i64>);

impl AmountSet {
    /// Build from authored pairs. A later pair on one currency replaces an
    /// earlier one, which is what `UNIQUE (line_id, currency)` says physically.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = (CurrencyCode, i64)>) -> Self {
        Self(values.into_iter().collect())
    }

    /// The value for one currency, or `None` when the market is uncovered.
    #[must_use]
    pub fn get(&self, currency: &CurrencyCode) -> Option<i64> {
        self.0.get(currency).copied()
    }

    /// Every covered currency, in a stable order.
    pub fn currencies(&self) -> impl Iterator<Item = &CurrencyCode> {
        self.0.keys()
    }

    /// Every pair, in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = (&CurrencyCode, i64)> {
        self.0.iter().map(|(c, v)| (c, *v))
    }

    /// Is the set empty? An amount-based line with no values covers no market
    /// at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A line's magnitude, with its type **declared** (D-08).
///
/// A sum rather than a pair of nullable fields, so "a percent value and an
/// amount set at once" and "neither" are both unconstructible. The store spends
/// a biconditional `CHECK` on the same rule, because the store has no sums.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Magnitude {
    /// A single basis-points value — currency-neutral.
    PercentBp(i64),
    /// Absolute money, per currency.
    Amount(AmountSet),
}

/// A line's adjustment: what it does to the running amount, and by how much.
///
/// **D-138 is normative and is encoded in the shape**: `Fixed` carries an
/// [`AmountSet`] rather than a [`Magnitude`], so a percentage `fixed` line does
/// not typecheck. `fixed` **replaces** the running line amount with an absolute
/// price at that layer; an additive reading would make it a duplicate of
/// `Markup`, which already expresses absolute money through
/// [`Magnitude::Amount`].
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adjustment {
    /// Adds its magnitude to the layer's input.
    Markup(Magnitude),
    /// Subtracts its magnitude from the layer's input.
    Discount(Magnitude),
    /// **Replaces** the running line amount. Always amount-based.
    Fixed(AmountSet),
}

impl Adjustment {
    /// The persisted / wire kind token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Markup(_) => "markup",
            Self::Discount(_) => "discount",
            Self::Fixed(_) => "fixed",
        }
    }

    /// The persisted `magnitude_kind` token.
    #[must_use]
    pub const fn magnitude_kind(&self) -> &'static str {
        match self {
            Self::Markup(Magnitude::PercentBp(_)) | Self::Discount(Magnitude::PercentBp(_)) => {
                "percent_bp"
            }
            // `fixed` shares this arm rather than carrying its own: D-138
            // makes it amount-based by construction, so "amount" here is the
            // same fact reached through a different variant.
            Self::Markup(Magnitude::Amount(_))
            | Self::Discount(Magnitude::Amount(_))
            | Self::Fixed(_) => "amount",
        }
    }

    /// The basis-points value, on a percent line only — the store's
    /// `adjustment_value` column.
    #[must_use]
    pub const fn percent_bp(&self) -> Option<i64> {
        match self {
            Self::Markup(Magnitude::PercentBp(v)) | Self::Discount(Magnitude::PercentBp(v)) => {
                Some(*v)
            }
            _ => None,
        }
    }

    /// The per-currency values, on an amount line only.
    #[must_use]
    pub const fn amounts(&self) -> Option<&AmountSet> {
        match self {
            Self::Markup(Magnitude::Amount(set)) | Self::Discount(Magnitude::Amount(set)) => {
                Some(set)
            }
            Self::Fixed(set) => Some(set),
            _ => None,
        }
    }

    /// Does this line's magnitude need per-currency coverage
    /// (`ADJUSTMENT_CURRENCY_NOT_COVERED`)?
    #[must_use]
    pub const fn is_amount_based(&self) -> bool {
        self.amounts().is_some()
    }

    /// Does this adjustment **replace** the running amount, voiding the layers
    /// it discards (D-138)?
    #[must_use]
    pub const fn replaces(&self) -> bool {
        matches!(self, Self::Fixed(_))
    }
}

/// One authored adjustment line.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayLine {
    /// The line's identity — stable across a copy-on-new-revision where the
    /// line is unchanged (D-92).
    pub line_id: Uuid,
    /// `(planId?, targetSku?, cohort?)`.
    pub key: LineKey,
    /// What it does, and by how much.
    pub adjustment: Adjustment,
}

/// The plans and SKUs an overlay's lines may target (`inst-plv-referential`).
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetRef {
    /// The plans in scope. Empty means the overlay targets nothing in
    /// particular, which only a list-default line can serve.
    pub plans: Vec<PlanId>,
}

impl TargetRef {
    /// Does this reference cover `plan`?
    #[must_use]
    pub fn covers(&self, plan: PlanId) -> bool {
        self.plans.contains(&plan)
    }
}

/// The overlay's own `[effectiveFrom, effectiveTo)` — **not** a price-row key
/// axis, because an overlay is not a price row.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayInterval {
    /// Inclusive start, or `None` for "from the beginning of time".
    pub from: Option<DateTime<Utc>>,
    /// Exclusive end, or `None` for open-ended.
    pub to: Option<DateTime<Utc>>,
}

impl OverlayInterval {
    /// Does this interval intersect `other`?
    ///
    /// Half-open on both sides, so a `to` equal to the other's `from` is
    /// **adjacent and not overlapping** — the rule the window plane already
    /// holds, and the one an operator relies on to schedule a successor.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        let starts_before_other_ends = match (self.from, other.to) {
            (Some(from), Some(to)) => from < to,
            _ => true,
        };
        let other_starts_before_this_ends = match (other.from, self.to) {
            (Some(from), Some(to)) => from < to,
            _ => true,
        };
        starts_before_other_ends && other_starts_before_this_ends
    }
}

/// **`inst-plv-lines`' resolution rule, normative and adopted by Rating step 4.**
///
/// Within one overlay the **most-specific line wins** for the priced row —
/// `(planId, targetSku)` > `(planId)` > list-default — and exactly one line of a
/// list applies per row. The eligibility filter (D-78) runs **first**: a line is
/// a candidate only if it is eligible for the row's class and generation, and
/// the ranking then runs unchanged inside that eligible set.
///
/// It is **never `precedence`**: precedence orders *lists* against each other
/// and never lines inside one.
///
/// `None` means no line of this overlay applies to the row, which is an ordinary
/// outcome and not a failure — a `plan`-scoped list simply says nothing about a
/// plan it does not name.
#[must_use]
pub fn resolve_line<'a>(
    lines: &'a [OverlayLine],
    plan: PlanId,
    sku: Option<&TargetSku>,
    eligibility: PriceEligibility,
    cohort: Option<DateTime<Utc>>,
) -> Option<&'a OverlayLine> {
    lines
        .iter()
        .filter(|line| line.key.eligible_for(eligibility, cohort))
        .filter(|line| match line.key.plan_id() {
            None => true,
            Some(target) => target == plan,
        })
        .filter(|line| match line.key.target_sku() {
            None => true,
            Some(target) => sku == Some(target),
        })
        .max_by_key(|line| line.key.specificity())
}

/// One overlay revision's **content**, as an approval unit pins it (D-225).
///
/// `PlanShape`'s counterpart for this plane: the shape a reviewer is shown and the
/// shape `content_pin::overlay_content_hash` frames. It is a domain type and not the
/// repository's `OverlayRecord` because the pin is pure domain — the encoder may not
/// depend on a storage row's field list, or a column added for an unrelated reason
/// would silently re-freeze every pending unit in every tenant.
///
/// # `row_version` is deliberately not here
///
/// The record carries one and this shape does not. A pin exists to answer *"is this
/// the content the reviewer saw"*, and the row version is a **concurrency token**:
/// it moves when the content moves, so including it adds nothing, and it can move
/// for reasons the content did not — which is a pin that fails for a reason the
/// reviewer cannot see. That is the same argument `PlanShape` makes for excluding
/// its `evaluated_at` and its baseline, and the exclusion is asserted rather than
/// described by `overlay_repo_tests::the_pin_ignores_the_row_version`.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRevision {
    /// The overlay's identity.
    pub price_overlay_id: Uuid,
    /// Which revision of it.
    pub revision: u64,
    /// Its state — pinned, because approving a draft and approving a published
    /// revision are not the same act.
    pub lifecycle_state: OverlayLifecycle,
    /// Scope class and value.
    pub scope: ScopeSelector,
    /// Its precedence within the class.
    pub precedence: i32,
    /// Its own dating.
    pub interval: OverlayInterval,
    /// Its declared tax basis.
    pub tax_basis: TaxBasis,
    /// Its exposure flag.
    pub disclosure: Disclosure,
    /// The plans its lines may target.
    pub target_ref: TargetRef,
    /// The whole line set — the money.
    pub lines: Vec<OverlayLine>,
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod overlay_tests;
