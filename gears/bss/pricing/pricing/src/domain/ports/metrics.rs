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
//! The **alarm** counter is the one place a name is passed as a string, and it
//! is bounded by a different mechanism — see [`PricingMetricsPort::alarm`].
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
    /// The plan has no published version at all, so there is nothing to read.
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
/// **Only Slice 4's two are declared so far**, and that is the honest state
/// rather than a stub: this module opens the plane, and each slice adds its own
/// alarms beside these when it wires them. A variant with no emitter would be a
/// roster entry claiming coverage that does not exist — the shape D-232 records
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
}

impl PricingAlarm {
    /// Every alarm this gear can currently raise.
    pub const ALL: &'static [Self] = &[Self::TaxNotSellableGaActive, Self::TaxReadinessDivergent];

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

    /// The live count of published tax-inclusive rows awaiting Tax Engine GA
    /// (`pricing_tax_not_sellable_ga`, §10 — the gauge §7's Info alarm watches).
    ///
    /// A **gauge**, observed rather than accumulated: the question is how many
    /// markets are gated *now*, and the number falls when the engine GAs and the
    /// affected plans re-publish. A counter would only ever rise and would answer
    /// "how many were ever gated", which is not the backlog anyone is managing.
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
