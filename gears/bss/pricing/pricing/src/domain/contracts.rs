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

use std::fmt;

use toolkit_macros::domain_model;

use crate::domain::plan_shape::PlanShape;
use crate::domain::scope_key::ChargeKind;
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

/// Slice 6's registered set over one publish subject.
#[must_use]
pub fn consumer_contract_rules() -> ValidationPipeline<PlanShape> {
    ValidationPipeline::new().with_rule(Box::new(BillingTimingPresent))
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
