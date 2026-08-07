//! Typed metrics port for the BSS pricing catalog.
//!
//! # The plane this opens, and why it is a port rather than a call
//!
//! The design set declares **28 alarms** and a metric surface spanning all
//! twelve slices, and until now this gear emitted **nothing**: a grep for
//! `counter!`, `gauge!` or `metrics::` over `src/` returned no hit. That is the
//! gap register entry `T-17` records. This is its foundation — not a Slice-4
//! convenience, and deliberately shaped so each later slice adds its own
//! instruments beside these rather than inventing a second way to.
//!
//! Nothing here is invented. Seven gears in this workspace already carry the
//! same `domain/ports/metrics.rs` + `infra/metrics` pair — ledger, credstore,
//! mini-chat, account-management, authn-resolver, oagw, usage-collector — and
//! the ledger's own adapter names it *"the canonical RBAC pattern"*. The choice
//! here was to follow it, which is why this module reads like its siblings.
//!
//! # Label values come from closed enums, and that is a cardinality bound
//!
//! Every label a caller can influence is an `as_str()` on a `#[domain_model]`
//! enum, so the series count is fixed at compile time. A `&str` label taken from
//! a request is an unbounded-cardinality hazard: a caller mints a new
//! time-series per distinct value and the scrape target grows without limit.
//! The ledger spends a whole allow-list on exactly that problem for one
//! caller-supplied `reason_code`; the cheaper answer, available here because
//! every label this gear reports is a *rule outcome* rather than caller input,
//! is to have no `&str` labels at all.
//!
//! The **alarm** counter is the one whose label is a declared *name* rather than
//! a rule outcome — the one place a design document would tempt a caller to pass
//! a `&str`. It is bounded the same way as the rest: by an enum. See
//! [`PricingMetricsPort::alarm`].
//!
//! # A no-op is the default, and it is the safe one
//!
//! [`NoopPricingMetrics`] satisfies the port and does nothing. It is what every
//! unit test and every construction before an exporter is wired holds, so a
//! missing exporter can never be the reason a publish fails. Instruments in the
//! adapter are likewise no-ops until the host installs a meter provider, so
//! emitting is always cheap and always safe.

use toolkit_macros::domain_model;

/// Why a base-price preview refused to answer
/// (`pricing_preview_failclosed_total{reason}`, §10).
///
/// The three are not one bucket, because they send an operator to three
/// different places: a market nobody authored, a plan that has never published,
/// and a request that named no market at all. A single `failclosed` count would
/// tell them the preview is refusing without telling them whose fault it is.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFailClosed {
    /// The plan publishes no row on the requested `(currency, region)` — the
    /// `inst-mc-nofx` refusal. A catalog gap: somebody has to author the row.
    MarketAbsent,
    /// No version of the tenant's catalog is readable, so there is nothing to
    /// quote on any market.
    ///
    /// **Wider than its name**, and that is recorded rather than hidden: the
    /// frontier read answers `None` both for a tenant that has never published
    /// and for one whose only projections are later than the pin — published, not
    /// yet warmed by the projector. The remediations differ (publish, versus
    /// wait), and separating them needs a fact the frontier read does not return.
    /// `T-17`.
    NoPublishedVersion,
    /// The request named no market, or a malformed one. A caller fault.
    MarketNotNamed,
}

impl PreviewFailClosed {
    /// Every reason, so a dashboard can enumerate the series.
    pub const ALL: &'static [Self] = &[
        Self::MarketAbsent,
        Self::NoPublishedVersion,
        Self::MarketNotNamed,
    ];

    /// Stable `snake_case` label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MarketAbsent => "market_absent",
            Self::NoPublishedVersion => "no_published_version",
            Self::MarketNotNamed => "market_not_named",
        }
    }
}

/// Which of §3's enumerated currency-binding configurations blocked a publish
/// (`pricing_currency_binding_blocks_total{case}`, §10).
///
/// The three cases §3 enumerates, kept apart because §3 enumerates them: one is
/// a plan's own add-on composition and two are a bundle's, and they are
/// remediated by different people.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrencyBindingCase {
    /// Case (i): a required add-on or override target missing a market the base
    /// plan sells (`inst-cb-addon`).
    RequiredAddon,
    /// Case (ii): a `sum_of_parts` bundle whose components do not cover every
    /// currency the bundle sells (`inst-cb-bundle-sum`).
    BundleSumOfParts,
    /// Case (iii): an `own_price` bundle whose components do not each carry a
    /// row in every currency it sells (`inst-cb-bundle-own`).
    BundleOwnPrice,
}

impl CurrencyBindingCase {
    /// Every case.
    pub const ALL: &'static [Self] = &[
        Self::RequiredAddon,
        Self::BundleSumOfParts,
        Self::BundleOwnPrice,
    ];

    /// Stable `snake_case` label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequiredAddon => "required_addon",
            Self::BundleSumOfParts => "bundle_sum_of_parts",
            Self::BundleOwnPrice => "bundle_own_price",
        }
    }
}

/// An alarm's severity (the `severity` label on `pricing_alarm_total`).
///
/// The three the design set actually uses across its alarm tables. It is a
/// closed enum for the module doc's reason, and because a severity is a property
/// of the *declaration* rather than of the occurrence — an alarm that could
/// report its own severity per firing would be one nobody can alert on.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmSeverity {
    /// Visibility only — a backlog worth watching, not a fault.
    Info,
    /// Something is wrong and someone should look.
    Warn,
    /// An invariant the catalog promises is broken.
    Critical,
}

impl AlarmSeverity {
    /// Every severity.
    pub const ALL: &'static [Self] = &[Self::Info, Self::Warn, Self::Critical];

    /// Stable `snake_case` label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }
}

/// Every alarm this gear may raise — the closed universe of the `alarm` label.
///
/// # Why an enum and not the alarm's name as a string
///
/// The design set declares 28 alarm names across twelve slices, and they are
/// *declarations*: a name no document names is not an alarm, it is a typo that
/// silently mints a new series nobody alerts on. An enum makes the roster a
/// compile-time fact and makes adding one a deliberate edit here — which is also
/// what lets a later slice add its alarms without inventing a second mechanism.
///
/// **Four are declared: Slice 4's two, and Foundation's two Criticals** — the
/// latter added by D-238 after the module that raises them was found still
/// arguing that this gear had no alarm plane, months after Slice 4 opened it.
/// Each slice adds its own alarms beside these when it wires them. A variant with
/// no emitter would be a roster entry claiming coverage that does not exist — the
/// shape D-232 records
/// one plane over, where a trigger answers `true` on the strength of a
/// constructor nobody calls.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingAlarm {
    /// `pricing.tax.not_sellable_ga_active` (Info, §7) — published tax-inclusive
    /// rows awaiting Tax Engine GA. Visibility for the C3 backlog, which the PRD
    /// risk table expects to stand for an estimated eight months.
    TaxNotSellableGaActive,
    /// `pricing.tax.readiness_divergent` (Warn, §7, D-01) — a tenant-declared
    /// readiness marker disagreeing with the Tax Engine's registry after GA.
    ///
    /// Declared here and **not yet raised**: the post-GA reconciliation it
    /// belongs to is part of the future Tax Engine contract, and this gear has
    /// no engine to reconcile against. It is in the roster because §7 names it
    /// and because the module doc's rule is that a slice adds its alarms when it
    /// wires them — this one's wiring is a contract, not code.
    TaxReadinessDivergent,
    /// `pricing.catalogversion.commit_overdue` (Critical, `01-foundation.md`
    /// §3.6) — a pending `CatalogVersion` ref the registry has not answered for
    /// past the max batching-delay SLO.
    CatalogVersionCommitOverdue,
    /// `pricing.readmodel.pin_eligibility_overdue` (Critical,
    /// `01-foundation.md` §4.4) — a tenant's pin frontier has not advanced
    /// inside the SLO while something is holding it.
    ReadModelPinEligibilityOverdue,
}

impl PricingAlarm {
    /// Every alarm this gear can currently raise.
    ///
    /// **The last two arrived late and the reason is worth keeping** (D-238).
    /// `readmodel_warm` has raised them as bare `tracing::error!` since it was
    /// written, on the argument that *"this gear has no metrics or alarm facility
    /// at all"* — true when written, falsified by Slice 4 opening this plane, and
    /// left standing afterwards. The consequence was sharper than the wording:
    /// these two are the gear's **only Critical** alarms, and they were the only
    /// two not on its alarm plane, so the estate's alerting saw every Info and
    /// Warn this gear raises and neither of the two that page somebody.
    pub const ALL: &'static [Self] = &[
        Self::TaxNotSellableGaActive,
        Self::TaxReadinessDivergent,
        Self::CatalogVersionCommitOverdue,
        Self::ReadModelPinEligibilityOverdue,
    ];

    /// The alarm's declared name, exactly as the design set spells it.
    ///
    /// The **dotted** form, not a `snake_case` rewrite: an operator greps the
    /// design document for `pricing.tax.readiness_divergent` and has to find the
    /// series. A label that renamed it would make the document and the dashboard
    /// two vocabularies.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaxNotSellableGaActive => "pricing.tax.not_sellable_ga_active",
            Self::TaxReadinessDivergent => "pricing.tax.readiness_divergent",
            Self::CatalogVersionCommitOverdue => "pricing.catalogversion.commit_overdue",
            Self::ReadModelPinEligibilityOverdue => "pricing.readmodel.pin_eligibility_overdue",
        }
    }

    /// The severity §7 declares for this alarm.
    ///
    /// A property of the alarm rather than a parameter, so two firings of one
    /// alarm cannot disagree about how urgent it is.
    #[must_use]
    pub const fn severity(self) -> AlarmSeverity {
        match self {
            Self::TaxNotSellableGaActive => AlarmSeverity::Info,
            Self::TaxReadinessDivergent => AlarmSeverity::Warn,
            Self::CatalogVersionCommitOverdue | Self::ReadModelPinEligibilityOverdue => {
                AlarmSeverity::Critical
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The port.
// ---------------------------------------------------------------------------

/// What the catalog reports about itself.
///
/// Held as `Arc<dyn PricingMetricsPort>` by the surfaces that emit, so a test
/// holds [`NoopPricingMetrics`] and production holds the `OTel` adapter without
/// either knowing about the other.
pub trait PricingMetricsPort: Send + Sync + 'static {
    /// One base-price preview refused to answer
    /// (`pricing_preview_failclosed_total{reason}`, §10).
    fn preview_failclosed(&self, reason: PreviewFailClosed);

    /// One publish blocked by the single-currency-per-invoice binding
    /// (`pricing_currency_binding_blocks_total{case}`, §10).
    ///
    /// Counted **per blocking case**, not per offending component: §10 labels it
    /// `case`, and a plan whose four add-ons all miss a market is one authoring
    /// mistake rather than four.
    fn currency_binding_block(&self, case: CurrencyBindingCase);

    /// Publish the **catalog-wide** count of markets carrying published
    /// tax-inclusive rows, which are not sellable until Tax Engine GA
    /// (`pricing_tax_not_sellable_ga`, §10).
    ///
    /// A **gauge**, observed rather than accumulated: the question is how many
    /// markets are gated *now*, and the number falls when the engine GAs and the
    /// affected plans re-publish. A counter would only ever rise and would answer
    /// "how many were ever gated", which is not the backlog anyone is managing.
    ///
    /// **Publishes rather than records** (D-246), and the caller has to mean it:
    /// the adapter's instrument is an *observable* gauge, so this stores the value
    /// for the exporter's next collection instead of emitting a point. A caller
    /// that knows only its own plan's contribution must **not** call this — that
    /// was the `[C]` `8c5e10075` removed, where an un-dimensioned series became
    /// last-writer-wins across every plan and tenant and one tax-exclusive plan
    /// cleared §7's alarm over five markets that stayed gated. The only honest
    /// caller is one holding the whole catalog's answer;
    /// `price_repo::gated_markets` is how that answer is obtained.
    ///
    /// §7's Info alarm rides the **counter** rather than this, for the same
    /// reason: a counter is well defined from one plan and this is not.
    fn tax_not_sellable_ga(&self, count: i64);

    /// One alarm firing (`pricing_alarm_total{alarm,severity}`).
    ///
    /// **One rollup counter for all 28**, which is the ledger's arrangement and
    /// its argument: an alerting rule filters by label, so a counter per alarm
    /// would be 28 instruments carrying no more information than one with a
    /// bounded label — and each new alarm would be a new instrument name for a
    /// dashboard to learn.
    fn alarm(&self, alarm: PricingAlarm);
}

/// The port doing nothing — the safe default.
///
/// Not `#[cfg(test)]`: it is what `module.rs` holds before an exporter exists
/// and what any caller constructing a service outside the gear lifecycle gets,
/// so it has to be available in a release build.
#[domain_model]
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopPricingMetrics;

impl PricingMetricsPort for NoopPricingMetrics {
    fn preview_failclosed(&self, _reason: PreviewFailClosed) {}
    fn currency_binding_block(&self, _case: CurrencyBindingCase) {}
    fn tax_not_sellable_ga(&self, _count: i64) {}
    fn alarm(&self, _alarm: PricingAlarm) {}
}
