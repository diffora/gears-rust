//! Slice 6's consumer contracts: the fields a downstream system computes from
//! (`design/06-consumer-contracts.md`).
//!
//! The catalog publishes **inputs**; the math and the enforcement live
//! downstream. Every rule here is therefore a presence/consistency statement
//! about what a publish freezes, never a computation over it — `inst-rc-nocompute`
//! is the slice's own name for that boundary and it is why no rule in this
//! module reads an amount.
//!
//! # Why the subject is the [`PlanShape`] and not a [`PriceRow`]
//!
//! Slice 3's row rules judge a [`PriceRow`](crate::domain::price_row::PriceRow),
//! which deliberately carries no identity and none of the columns this slice
//! owns: `billing_timing` is a column of the
//! [`PriceRecord`](crate::domain::price_record::PriceRecord) that wraps a row,
//! not of the row. A rule here could not read its own field from Slice 3's
//! subject, so the subject is the plan and each rule walks `shape.rows`.
//!
//! That is also what the market-uniformity rule needs (`inst-pi-uniform`): a
//! statement about a *set* of rows crossed with a set of markets has no
//! row-local form at all.

use std::collections::BTreeMap;
use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::PlanShape;
use crate::domain::price_record::PriceRecord;
use crate::domain::scope_key::{ChargeKind, PriceEligibility, Region};
use crate::domain::validation::{ValidationPipeline, ValidationReport, ValidationRule};

/// The day of the month a `fixed_day` anchor lands on, 1–31.
///
/// A newtype rather than a bare `u8` because the range is the whole of its
/// meaning and §6 states it as a constraint (`anchor_day BETWEEN 1 AND 31`) that
/// no `CHECK` expresses on either engine — see
/// [`m20260802_000050`](crate::infra::storage::migrations::m20260802_000050_add_pricing_price_proration_contract)
/// for why. Refusing the value at construction is the stronger form of the same
/// statement: a column can hold only what this renders.
///
/// **29, 30 and 31 are legal and are not a mistake.** K2 makes a day past the
/// month's length anchor on the **last day of the month**, with the anchor day
/// preserved across periods (31 → 28 → 31, an independent per-period clamp with
/// no drift). Refusing them here would make February the shortest legal anchor
/// and quietly forbid the month-end billing K2 exists to define.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnchorDay(u8);

impl AnchorDay {
    /// The day, if it is one a month can have.
    ///
    /// # Errors
    /// [`AnchorDayOutOfRange`] for `0` or anything past `31`.
    pub const fn new(day: u8) -> Result<Self, AnchorDayOutOfRange> {
        if day >= 1 && day <= 31 {
            Ok(Self(day))
        } else {
            Err(AnchorDayOutOfRange(day))
        }
    }

    /// The day as stored.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl fmt::Display for AnchorDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An anchor day outside the 1–31 a month can offer.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorDayOutOfRange(pub u8);

impl fmt::Display for AnchorDayOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "anchor_day must be between 1 and 31; got {}. A day past the \
             month's length is legal and anchors on the last day of the month \
             (K2), so the bound is the calendar's, not the shortest month's",
            self.0
        )
    }
}

impl std::error::Error for AnchorDayOutOfRange {}

/// Where a subscription's billing cycle boundary falls (K2, §6).
///
/// `FixedDay` **carries its day**, for the reason
/// [`Frequency::CustomEveryN`](crate::domain::plan_shape::Frequency) carries its
/// interval: §6 gives the policy and `anchor_day` as one fact in two columns,
/// and the two spellings a flat pair admits — a `fixed_day` with no day, a day
/// beside `calendar_month` — are both unpublishable. Holding them together makes
/// the pairing structural, so `inst-pi-anchor` needs no rule for it and no
/// engine needs a `CHECK` it cannot express.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingAnchorPolicy {
    /// The first of the calendar month, UTC.
    CalendarMonth,
    /// The subscription's own start day, clamped per period under monthly
    /// granular cycles (K2, D-20).
    SubscriptionStart,
    /// A named day of the month, clamped to the month's last day when the month
    /// is shorter.
    FixedDay(AnchorDay),
}

impl BillingAnchorPolicy {
    /// The persisted / wire token, which never carries the day.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalendarMonth => "calendar_month",
            Self::SubscriptionStart => "subscription_start",
            Self::FixedDay(_) => "fixed_day",
        }
    }

    /// The day this policy anchors on, when it names one.
    #[must_use]
    pub const fn anchor_day(self) -> Option<AnchorDay> {
        match self {
            Self::FixedDay(day) => Some(day),
            Self::CalendarMonth | Self::SubscriptionStart => None,
        }
    }
}

impl fmt::Display for BillingAnchorPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixedDay(day) => write!(f, "fixed_day({day})"),
            other => f.write_str(other.as_str()),
        }
    }
}

/// The canonical proration basis (K1, §6).
///
/// **Owned here.** K1 makes this enum the one source Tariffs adopts verbatim and
/// Subscriptions computes from, and says any extension is a versioned contract
/// change — so a second spelling of this set anywhere is the enum-drift failure
/// class §1.2 exists to kill.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProrationBasis {
    /// Days actually in the period.
    CalendarDaysActual,
    /// A 30-day month regardless of the calendar's.
    CalendarDays30,
    /// Second-granular.
    BySecond,
    /// No partial unit: a started unit is a whole one.
    WholeUnit,
    /// No proration at all.
    None,
}

impl ProrationBasis {
    /// K1's set, whole, for the readers that map a stored token back.
    ///
    /// Spelled out rather than derived so that adding a member is a change this
    /// array records — K1 makes any extension a **versioned contract change**,
    /// and one that slipped in without touching a declared roster is exactly the
    /// drift `pricing.contracts.enum_drift` alarms on.
    pub const ALL: &'static [Self] = &[
        Self::CalendarDaysActual,
        Self::CalendarDays30,
        Self::BySecond,
        Self::WholeUnit,
        Self::None,
    ];

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalendarDaysActual => "calendar_days_actual",
            Self::CalendarDays30 => "calendar_days_30",
            Self::BySecond => "by_second",
            Self::WholeUnit => "whole_unit",
            Self::None => "none",
        }
    }

    /// Is this the basis that computes nothing?
    ///
    /// Read through here rather than matched at each site: `inst-pi-credit-none`
    /// is the only rule that cares which member this is, and a second `match`
    /// on the variant is a second place to forget it.
    #[must_use]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

impl fmt::Display for ProrationBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The three fields a recurring row publishes for Subscriptions' proration
/// (`inst-pi-required`), held together because they are required together.
///
/// One `Option<ProrationContract>` on the record rather than three independent
/// `Option`s: §3 step 1 makes all three REQUIRED on a recurring row and absence
/// fail publish, so the only states worth representing are "authored" and "not
/// authored". Three loose options admit six partial states that no rule can
/// report usefully and that the market-uniformity comparison would have to
/// compare field by field anyway.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProrationContract {
    /// Where the cycle boundary falls (K2).
    pub billing_anchor_policy: BillingAnchorPolicy,
    /// The canonical basis Subscriptions and Tariffs both read (K1).
    pub proration_basis: ProrationBasis,
    /// Whether a downgrade off this row is credit-eligible.
    ///
    /// **The governing value on a plan change is the *source* row's**, read from
    /// the subscription's frozen snapshot — never the target's and never the
    /// live catalog (`inst-pi-credit-source`). That is a rule about which
    /// snapshot a downstream reader picks, so the catalog's whole obligation is
    /// to publish the field per row and freeze it; nothing here chooses.
    pub credit_on_downgrade: bool,
}

/// A published recurring row carries no `billingTiming`
/// (`06-consumer-contracts.md` §3 `inst-bt-required`, §5).
///
/// The code is §5's verbatim. **Slice 2 names it too** and does not register it:
/// `plan_rules::cycle_shape`'s module doc records that `billingTiming` REQUIRED
/// on a recurring row is Slice 6's rule, cross-referenced and never
/// re-registered, while Slice 2 owns the opposite statement about a *setup* row
/// (`inst-cs-setup`, `SETUP_ROW_INVALID`). The two can never disagree because
/// the `chargeKind` axis makes no row both.
pub const BILLING_TIMING_MISSING: &str = "BILLING_TIMING_MISSING";

/// Every recurring row of the publish subject states its `billingTiming`.
///
/// **Recurring only, and every recurring row.** Usage and one-time rows do not
/// author the field — `inst-bt-usage` projects a constant for them — and a rule
/// that demanded it there would reject a row whose value is not the author's to
/// give. `existing_grandfathered` rows are *not* excluded: D-132's exclusion is
/// an argument about comparing a frozen generation against current rows, which
/// is the uniformity rule's problem and not this one. A grandfathered row that
/// published without the field would leave Billing deriving a deferral policy
/// for a live subscriber by heuristic, which is exactly what `inst-bt-frozen`
/// exists to prevent.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct BillingTimingPresent;

impl ValidationRule<PlanShape> for BillingTimingPresent {
    fn name(&self) -> &'static str {
        "inst-bt-required"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for record in &subject.rows {
            if !is_recurring(record.scope_key.charge_kind()) || record.billing_timing.is_some() {
                continue;
            }
            report.violate(
                BILLING_TIMING_MISSING,
                record.price_id.to_string(),
                "a recurring row MUST state its billingTiming (advance | arrears): Billing \
                 derives its deferral policy from the frozen value and never from a heuristic, so \
                 an absent one is a row no consumer can defer correctly (inst-bt-required, \
                 inst-bt-frozen)"
                    .to_owned(),
            );
        }
    }
}

/// A recurring row publishes without its proration inputs
/// (`06-consumer-contracts.md` §3 `inst-pi-required`, §5).
pub const PRORATION_INPUTS_MISSING: &str = "PRORATION_INPUTS_MISSING";

/// `creditOnDowngrade = true` on a row whose basis computes nothing
/// (`inst-pi-credit-none`, §5).
pub const PRORATION_INPUTS_CONTRADICTORY: &str = "PRORATION_INPUTS_CONTRADICTORY";

/// The recurring rows of one plan-market disagree on the contract
/// (D-123 as scoped by D-132, `inst-pi-uniform`, §5).
pub const PRORATION_CONTRACT_MIXED_MARKET: &str = "PRORATION_CONTRACT_MIXED_MARKET";

/// Every recurring row states all three proration inputs (`inst-pi-required`).
///
/// The domain type makes "all three or none" the only representable pair, so
/// this rule has exactly one thing to say: the set is absent on a row that owes
/// it. What it does **not** say is anything about the values — `inst-pi-enum`
/// (K1's five bases) and the `fixed_day`/`anchor_day` pairing of
/// `inst-pi-anchor` are both discharged by [`ProrationBasis`] and
/// [`BillingAnchorPolicy`] being unable to hold a violation, which is the
/// stronger form of the same statement and the one `publish::rules` already
/// takes for the money rules.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProrationInputsPresent;

impl ValidationRule<PlanShape> for ProrationInputsPresent {
    fn name(&self) -> &'static str {
        "inst-pi-required"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for record in &subject.rows {
            if !is_recurring(record.scope_key.charge_kind()) || record.proration_contract.is_some()
            {
                continue;
            }
            report.violate(
                PRORATION_INPUTS_MISSING,
                record.price_id.to_string(),
                "a recurring row MUST publish billingAnchorPolicy, prorationBasis and \
                 creditOnDowngrade: Subscriptions computes its proration from the frozen values \
                 and the catalog substitutes no defaults, so an absent set is a row no consumer \
                 can prorate (inst-pi-required)"
                    .to_owned(),
            );
        }
    }
}

/// A credited downgrade needs a basis to compute the partial period with
/// (`inst-pi-credit-none`).
///
/// The contradiction is `creditOnDowngrade = true` beside
/// `prorationBasis = none`: the row promises a credit and denies the arithmetic
/// that would size it. Both halves are individually legal, which is why this is
/// a rule and not a type — `none` is a real basis (a plan that never prorates)
/// and `true` is a real flag; only the pair is unpublishable.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProrationCreditHasBasis;

impl ValidationRule<PlanShape> for ProrationCreditHasBasis {
    fn name(&self) -> &'static str {
        "inst-pi-credit-none"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for record in &subject.rows {
            let Some(contract) = record.proration_contract else {
                continue;
            };
            if !(contract.credit_on_downgrade && contract.proration_basis.is_none()) {
                continue;
            }
            report.violate(
                PRORATION_INPUTS_CONTRADICTORY,
                record.price_id.to_string(),
                "creditOnDowngrade = true with prorationBasis = none is a contradiction: the row \
                 grants a credit for a surrendered part-period and states no basis to compute \
                 that part with (inst-pi-credit-none)"
                    .to_owned(),
            );
        }
    }
}

/// One proration/anchor contract per plan-market (D-123, scoped by D-132).
///
/// **A subscription is one cycle clock** — D-110's "an invoice is one document"
/// applied to the time axis. Phase is a scope-key axis, so under D-15 a phased
/// plan carries one recurring row per charging phase per market, each with its
/// own anchor as authored; the consuming side reads **one** value, because
/// Subscriptions' `billingAnchor` is a single field on the aggregate. Nothing
/// related the N authored values to the one consumed value, so an intro-pricing
/// plan anchoring `subscription_start` on the intro row and `fixed_day(1)` on
/// the terminal row published both into one frozen snapshot with no rule saying
/// which sets the boundary. This rule makes the question not arise: the phase
/// axis becomes cycle-clock-neutral by construction.
///
/// **Per market, not per plan.** Anchoring EU on the 1st and US on signup day
/// stays legal, exactly as D-110 lets two markets differ on tax basis.
///
/// **`existing_grandfathered` generations are excluded before the grouping**,
/// not filtered out of a finding afterwards — [`MarketBasisUniform`]'s reason
/// and D-132's: an immutable, never-superseded generation must not even
/// contribute a value to compare against, or one cutover would permanently
/// freeze the market's cycle clock and every later publish would fail on a row
/// nobody can fix. A grandfathered subscriber reads these fields from its own
/// frozen snapshot, so "a subscription is one cycle clock" still holds per
/// subscription.
///
/// [`MarketBasisUniform`]: crate::domain::tax_display::MarketBasisUniform
///
/// **`billingTiming` is exempt** and is not read here: it is deliberately
/// per-row, because a hybrid mixes an `advance` base with an `arrears` usage
/// line (`inst-bt-usage`) and Billing consumes it per line rather than as a
/// subscription-level clock.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct ProrationContractMarketUniform;

impl ValidationRule<PlanShape> for ProrationContractMarketUniform {
    fn name(&self) -> &'static str {
        "inst-pi-uniform"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        // The contract is carried **beside** the row rather than left on it as
        // an `Option`, so nothing below has to re-assert that a row in this map
        // has one. A row with no contract at all is `inst-pi-required`'s
        // finding: admitting it here would report one omission under two codes
        // and send the author to remediate it in two places.
        let mut markets: BTreeMap<Market, Vec<ContractedRow<'_>>> = BTreeMap::new();
        for record in &subject.rows {
            if !is_recurring(record.scope_key.charge_kind())
                || !in_uniformity_set(record.scope_key.price_eligibility())
            {
                continue;
            }
            let Some(contract) = record.proration_contract else {
                continue;
            };
            markets
                .entry((
                    record.scope_key.currency().clone(),
                    record.scope_key.region().clone(),
                ))
                .or_default()
                .push((record, contract));
        }

        for ((currency, region), rows) in markets {
            let Some(((_, reference), rest)) = rows.split_first() else {
                continue;
            };
            if !rest.iter().any(|(_, c)| c != reference) {
                continue;
            }

            let diverges = |field: fn(&ProrationContract, &ProrationContract) -> bool| {
                rest.iter().any(|(_, c)| field(c, reference))
            };
            let mut fields = Vec::new();
            if diverges(|c, r| c.billing_anchor_policy != r.billing_anchor_policy) {
                fields.push("billingAnchorPolicy");
            }
            if diverges(|c, r| c.proration_basis != r.proration_basis) {
                fields.push("prorationBasis");
            }
            if diverges(|c, r| c.credit_on_downgrade != r.credit_on_downgrade) {
                fields.push("creditOnDowngrade");
            }

            // Every row of the market is named, not only the ones that differ
            // from whichever happened to be first: an operator told "these two
            // disagree" still has to find what the market's contract is, and
            // rendering each row beside its own values answers that in one read.
            let sides: Vec<String> = rows
                .iter()
                .map(|(record, c)| {
                    format!(
                        "{}: billingAnchorPolicy={}, prorationBasis={}, creditOnDowngrade={}",
                        record.price_id,
                        c.billing_anchor_policy,
                        c.proration_basis,
                        c.credit_on_downgrade
                    )
                })
                .collect();

            report.violate(
                PRORATION_CONTRACT_MIXED_MARKET,
                format!("{}/{region}", currency.as_str()),
                format!(
                    "recurring rows of this plan on market {}/{region} disagree on {} - {}. A \
                     subscription is one cycle clock, and phase is a scope-key axis, so a phased \
                     plan's rows on one market must carry one contract or nothing says which of \
                     them sets the boundary (D-123). Grandfathered generations are excluded from \
                     this set (D-132)",
                    currency.as_str(),
                    fields.join(", "),
                    sides.join(" | ")
                ),
            );
        }
    }
}

/// Slice 6's registered set over one publish subject.
#[must_use]
pub fn consumer_contract_rules() -> ValidationPipeline<PlanShape> {
    ValidationPipeline::new()
        .with_rule(Box::new(BillingTimingPresent))
        .with_rule(Box::new(ProrationInputsPresent))
        .with_rule(Box::new(ProrationCreditHasBasis))
        .with_rule(Box::new(ProrationContractMarketUniform))
}

/// The `(currency, region)` pair D-123 scopes uniformity to. Named because the
/// rule groups by it and `tax_display`'s D-110 sibling groups by the same pair.
type Market = (CurrencyCode, Region);

/// One row of a market beside the contract it published. The contract is not an
/// `Option` here: rows without one are `inst-pi-required`'s finding and never
/// enter the grouping.
type ContractedRow<'a> = (&'a PriceRecord, ProrationContract);

/// Is this row's eligibility class inside D-123's uniformity row set?
///
/// The two classes D-123 names, and not the third. An `existing_grandfathered`
/// generation is immutable and never superseded, so a divergence it carries can
/// never be remediated — including it would let one cutover permanently freeze
/// the market's cycle clock and fail every later publish on a row nobody can
/// fix (D-132). It is excluded **before** the grouping rather than filtered out
/// of a finding, so it cannot decide the market's verdict either.
fn in_uniformity_set(eligibility: PriceEligibility) -> bool {
    matches!(
        eligibility,
        PriceEligibility::AllSubscriptions | PriceEligibility::NewSubscriptionsOnly
    )
}

/// Is this row one the recurring-row contracts apply to?
fn is_recurring(charge_kind: ChargeKind) -> bool {
    matches!(charge_kind, ChargeKind::Recurring)
}

/// The `billingTiming` a row **publishes**, which on every kind but `recurring`
/// is a constant the author never gave (`inst-bt-usage`).
///
/// Usage is implicitly `arrears` — its quantity is not known until the period
/// closes — and a one-time or setup row is charged at the event, which is
/// `advance`. A hybrid therefore publishes `advance` on its base line and
/// `arrears` on its usage line, which is the mix `inst-bt-usage` sanctions and
/// the reason `inst-pi-uniform` exempts this field from market uniformity.
///
/// **The constant wins over anything in the column**, and that is the point: it
/// is a projection, not a default. Were an authored value allowed to displace
/// it, Billing's deferral on a usage line would depend on whether someone had
/// typed into a column the design says is not theirs to author — and no rule
/// would report the difference, because there is nothing to report about a
/// field that is not authored. Slice 2 already refuses the value outright on a
/// setup row (`inst-cs-setup`, `SETUP_ROW_INVALID`); this closes the same gap
/// on the read side for every non-recurring kind at once.
///
/// The tokens are the **stored** spelling (`advance` / `arrears`), which is what
/// `chk_pricing_price_billing_timing` admits and what a recurring row already
/// projects. See this module's note in the hand-back on the design set's
/// `in_advance` / `in_arrears` prose.
#[must_use]
pub fn published_billing_timing(charge_kind: ChargeKind, authored: Option<&str>) -> Option<&str> {
    match charge_kind {
        ChargeKind::Recurring => authored,
        ChargeKind::Usage => Some(BILLING_TIMING_ARREARS),
        ChargeKind::OneTime | ChargeKind::OneTimeSetup => Some(BILLING_TIMING_ADVANCE),
    }
}

/// Charged at the start of the period it covers.
pub const BILLING_TIMING_ADVANCE: &str = "advance";

/// Charged after the period it covers has closed.
pub const BILLING_TIMING_ARREARS: &str = "arrears";

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod contracts_tests;
