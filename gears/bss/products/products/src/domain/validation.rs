//! The fail-closed validation pipeline.
//!
//! This is the donor's shape, adopted after measuring it: `gears/bss/pricing`
//! registers rules and collects a report, and its codes are constants on the
//! rules that raise them. The Foundation's addition is the **phase**: a run
//! stops at the first failing phase and collects violations per field *within*
//! that phase into one rejection, because the audit row carries a single
//! `error_code` and collecting across phases would produce more codes than the
//! row can record (P-D-33, P-D-37).
//!
//! Authorization is **not** a phase. It is a pre-pipeline gate, which is the
//! only order in which a denied caller neither consumes an idempotency key nor
//! writes a claim row (P-D-30).

use core::fmt;

use toolkit_macros::domain_model;

/// The ordered phases of a pipeline run.
///
/// @cpt-cf-bss-products-algo-pipeline
///
/// Declaration order **is** execution order, and the derived `Ord` is what the
/// run relies on, so a phase inserted in the wrong place changes behaviour
/// rather than merely reading oddly.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Resolves `(tenant, endpoint, client key)`. The first phase of every
    /// mutating flow **that carries a key** — skipped, never failed, on a
    /// keyless request (P-D-34).
    Idempotency,
    /// `If-Match` on a head verb; the pinned revision at the publish door.
    Precondition,
    /// Types, formats, required-at-this-state, and the resolvability of every
    /// reference the payload carries.
    Shape,
    /// The edge list, bucket routing, the parent's terminal state and the
    /// subject's own — everything judged from the row as it now stands rather
    /// than from the payload (P-D-24).
    State,
    /// Uniqueness, reservation, containment.
    Identity,
    /// Each feature's contributed rules, in registration order.
    RegisteredValidators,
    /// Hosts any gated act, not publish alone, and passes trivially where the
    /// act is ungated (P-D-30, P-D-34).
    GovernanceGate,
}

impl Phase {
    /// The phases in execution order.
    #[must_use]
    pub const fn ordered() -> [Self; 7] {
        [
            Self::Idempotency,
            Self::Precondition,
            Self::Shape,
            Self::State,
            Self::Identity,
            Self::RegisteredValidators,
            Self::GovernanceGate,
        ]
    }
}

/// One finding against a candidate mutation.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The wire code this finding rides. Constants live on the rules that raise
    /// them; this is where the rule put its own.
    pub code: &'static str,
    /// The field or subject the finding is about, in the payload's own naming.
    pub subject: String,
    /// What is wrong, for a human reading the response.
    pub detail: String,
}

/// The aggregate of one phase's findings.
///
/// The rejection a caller receives carries every violation the phase collected;
/// the audit row records one code — the first, by the precedence the taxonomy
/// states for that phase (P-D-37).
#[domain_model]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    violations: Vec<Violation>,
}

impl ValidationReport {
    /// An empty report.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            violations: Vec::new(),
        }
    }

    /// Record a blocking finding.
    pub fn violate(
        &mut self,
        code: &'static str,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.violations.push(Violation {
            code,
            subject: subject.into(),
            detail: detail.into(),
        });
    }

    /// Whether the phase admitted the mutation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    /// Every finding, in the order the rules produced them.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// The single code the audit row records: the first collected.
    ///
    /// `None` on an empty report, which is the case where nothing is audited
    /// because nothing was refused.
    #[must_use]
    pub fn audit_code(&self) -> Option<&'static str> {
        self.violations.first().map(|v| v.code)
    }
}

impl fmt::Display for ValidationReport {
    /// Renders the blocking count. The detail belongs in the response envelope,
    /// not in a log line that would repeat it per rule.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} violation(s)", self.violations.len())
    }
}

/// One registered rule.
///
/// A rule reads the candidate it is given and appends its findings. It never
/// short-circuits the run, never mutates the subject, and **never reads another
/// rule's verdict** — which is what makes registration order an ordering of
/// output rather than of logic.
pub trait ValidationRule<S>: Send + Sync {
    /// The instruction id this rule answers to. Reported by
    /// [`ValidationPipeline::rule_names`] for observability only: attribution in
    /// a rejection rides the error code, never the rule name.
    fn name(&self) -> &'static str;

    /// Which phase this rule runs in.
    fn phase(&self) -> Phase;

    /// Evaluate against `subject`, appending to `report`.
    fn evaluate(&self, subject: &S, report: &mut ValidationReport);
}

/// The pipeline: an ordered set of rules over one subject type.
///
/// @cpt-cf-bss-products-algo-pipeline
/// @cpt-cf-bss-products-dod-validation-pipeline
/// @cpt-cf-bss-products-principle-registered-validators
/// @cpt-cf-bss-products-principle-fail-closed
///
/// Rules run phase by phase in [`Phase::ordered`]; within a phase they run in
/// registration order. **The run stops at the first failing phase**, so a
/// report is always one phase's findings and never a mixture.
#[domain_model]
pub struct ValidationPipeline<S> {
    rules: Vec<Box<dyn ValidationRule<S>>>,
}

impl<S> ValidationPipeline<S> {
    /// An empty pipeline.
    ///
    /// An empty pipeline admits everything, which is why the Foundation
    /// registers its own shape and identity rules rather than leaving the base
    /// set to whichever feature happens to load first.
    #[must_use]
    pub const fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Register a rule. Order within a phase is preserved and is the order
    /// findings appear in.
    #[must_use]
    pub fn with_rule(mut self, rule: Box<dyn ValidationRule<S>>) -> Self {
        self.rules.push(rule);
        self
    }

    /// The registered rule names, in registration order.
    #[must_use]
    pub fn rule_names(&self) -> Vec<&'static str> {
        self.rules.iter().map(|rule| rule.name()).collect()
    }

    /// Run the pipeline, stopping at the first failing phase.
    ///
    /// Returns the failing phase and its report, or `None` when every phase
    /// admitted the mutation.
    #[must_use]
    pub fn run(&self, subject: &S) -> Option<(Phase, ValidationReport)> {
        for phase in Phase::ordered() {
            let mut report = ValidationReport::new();
            for rule in self.rules.iter().filter(|r| r.phase() == phase) {
                rule.evaluate(subject, &mut report);
            }
            if !report.is_empty() {
                return Some((phase, report));
            }
        }
        None
    }
}

impl<S> Default for ValidationPipeline<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;
