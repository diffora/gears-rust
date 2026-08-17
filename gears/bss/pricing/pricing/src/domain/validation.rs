//! The fail-closed validation pipeline framework.
//!
//! The Foundation owns the *mechanism*; slices own the *rules*. A slice
//! registers [`ValidationRule`] implementations and the pipeline runs the whole
//! set, collecting every outcome rather than stopping at the first failure —
//! the aggregate report is what makes a plan remediable in one pass.
//!
//! Two properties are normative rather than convenient
//! (`design/01-foundation.md` §4.2):
//!
//! - **A single blocking violation blocks the publish.** There is no severity
//!   below "blocking" that still publishes; advisory findings are warnings and
//!   carry no veto.
//! - **The same rule set runs twice**: as a pre-check at submit, and again
//!   inside the publish-commit transaction. Approval approves *content*; the
//!   commit re-validates *state*, because the world moved between the two. A
//!   commit-time failure voids the approval and returns the subject to draft
//!   with the report — so a rule must be pure with respect to the state it is
//!   handed, and must not carry results between the two runs.

use std::fmt;

use toolkit_macros::domain_model;

/// When a violation can first be judged — D-312.
///
/// The rule set runs in full at the publish pre-check and again inside the
/// publish commit; that is unchanged and normative (§4.2). What this adds is a
/// second, *earlier* place a **subset** of the same violations is also judged:
/// the price authoring write.
///
/// The subset is not "the cheap rules" or "the important ones". A violation is
/// [`Stage::Write`] when **every operand of its fault is present in the request
/// and one of them is an immutable component of the scope key** — `chargeKind`
/// above all, the key being uneditable after create. Such a request is complete,
/// self-inconsistent and knowably unpublishable the instant it arrives, and the
/// only call that resolves it retracts the field just sent rather than adding
/// anything. Refusing it at the write therefore costs an author nothing they
/// could legitimately have wanted.
///
/// Everything else is [`Stage::Publish`], and that includes two families it is
/// tempting to sweep in:
///
/// - **An absent operand** — no `model_kind` yet, no bands yet, no amount yet.
///   These are completed by a later call, which is the multi-call authoring
///   §4.2 exists to protect.
/// - **Content contradicting content** — tier bands on a `flat` row. Both
///   operands are mutable, so `model_kind: graduated` resolves it by adding
///   intent rather than by retracting.
///
/// **The stage belongs to the violation, not to the rule** — with one exception,
/// named below because an unnamed exception to a rule stated this flatly is how the
/// next reader decides the code is wrong.
///
/// **Six of the eight** rules that emit a write-stage violation also emit
/// publish-stage ones, and `EVAL_POLICY_MISPLACED` is the code for both a key
/// contradiction and a content-against-content fault — so neither a marker on the
/// rule nor a filter keyed on the code can express the split without over-refusing.
/// The eight, measured 2026-08-17 rather than remembered (this census said "three of
/// the four" while five of seven was the fact, so re-derive it from
/// `violate_at_write` / [`ValidationReport::violate_at`] and attribute each call to
/// its rule rather than trusting this list):
///
/// - Both stages: `inst-la-fields`, `inst-mk-forbidden`, `inst-rv-attrs`,
///   `inst-ac-gate`, and the two per-instance rules below.
/// - Write stage only: `inst-mk-chargekind`, `inst-cmp-planname`.
///
/// **The exception: a rule may take its stage from the door that asks.** What a
/// marker cannot carry is *the* stage of a rule's violations; what a rule can carry
/// is a per-instance parameter, when **every** fault it emits is judgeable exactly
/// where any of them is — because then the answer is a property of the surface and
/// not of the fault. `inst-ph-row-attached` (D-342) and `inst-ph-graph` (2026-08-17
/// review) are that shape: both read the submitted phase set, which **is** the
/// request at the `phases` facet and is not an operand of the price-row write at all.
///
/// Three things keep the exception from becoming the marker this doc warns about:
/// the stage is a field on the instance rather than on the rule type, [`Default`] is
/// [`Stage::Publish`] so `plan_shape_rules` registers `::default()` and the aggregate
/// pipeline stays a publish pipeline, and each rule's own case asserts that the
/// pipeline's instance yields `write_stage_only() == None` over a subject the door's
/// instance refuses. Reach for it only with that third part: without it, "the door
/// decides" is indistinguishable from a rule that refuses everywhere.
#[domain_model]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stage {
    /// Judged at the authoring write **and** at publish.
    Write,
    /// Judged at publish only. The default: a new rule is publish-stage unless
    /// its author states otherwise, which is the safe direction — a rule wrongly
    /// left here refuses later than it could, and a rule wrongly moved to
    /// [`Stage::Write`] refuses an author's legitimate intermediate state.
    #[default]
    Publish,
}

/// A blocking rule failure. `code` is the machine-readable discriminator the
/// design set names (`TIER_BANDS_OVERLAP`, `SUPERSESSION_UNIT_MISMATCH`, …);
/// `subject` locates it for the author (a price id, a phase id, a scope key).
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Machine-readable rule code.
    pub code: String,
    /// What the violation is about — the row, band, phase or key at fault.
    pub subject: String,
    /// Human-readable detail for the authoring surface.
    pub detail: String,
    /// The earliest surface that can judge this fault (D-312).
    pub stage: Stage,
}

/// An advisory finding. Surfaced to the author, never a veto — a warning that
/// could block publish would be a violation, and calling it a warning would
/// hide a fail-closed rule behind a soft word.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    /// Machine-readable advisory code.
    pub code: String,
    /// What the advisory is about.
    pub subject: String,
    /// Human-readable detail for the authoring surface.
    pub detail: String,
}

/// The aggregate outcome of one pipeline run: every blocking violation plus
/// every advisory finding, in rule-registration order.
#[domain_model]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    /// Blocking failures. Non-empty means the publish does not happen.
    pub violations: Vec<Violation>,
    /// Advisory findings. Never affect the verdict.
    pub warnings: Vec<Advisory>,
}

impl ValidationReport {
    /// Does this report permit the publish to proceed?
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        self.violations.is_empty()
    }

    /// Merge another report into this one, preserving order.
    pub fn absorb(&mut self, other: Self) {
        self.violations.extend(other.violations);
        self.warnings.extend(other.warnings);
    }

    /// Record a blocking violation.
    pub fn violate(
        &mut self,
        code: impl Into<String>,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.violations.push(Violation {
            code: code.into(),
            subject: subject.into(),
            detail: detail.into(),
            stage: Stage::Publish,
        });
    }

    /// Record a blocking violation the **authoring write** can already judge.
    ///
    /// Use only where every operand of the fault is in the request and one of
    /// them is frozen by the scope key — see [`Stage`]. Reaching for this on a
    /// fault whose operands can still change is how multi-call authoring breaks,
    /// and no gate would catch it: the row simply stops saving.
    pub fn violate_at_write(
        &mut self,
        code: impl Into<String>,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.violations.push(Violation {
            code: code.into(),
            subject: subject.into(),
            detail: detail.into(),
            stage: Stage::Write,
        });
    }

    /// Record a blocking violation stamped with `stage`.
    ///
    /// For the rules whose stage is a **per-instance parameter** rather than a
    /// property of the fault — `inst-ph-row-attached` (D-342) and `inst-ph-graph`
    /// (2026-08-17 review), each emitting one sentence whichever door asks.
    /// Dispatching here rather than at a pair of `report.violate*` calls per fault
    /// is what keeps that true: the code, subject and detail an author reads cannot
    /// come to depend on which door refused them, and `inst-ph-graph` has three
    /// faults that would otherwise carry three copies of the dispatch.
    ///
    /// It is deliberately **not** a shorthand for [`Self::violate_at_write`]: a rule
    /// that knows its fault is write-judgeable says so directly, and passing a
    /// constant [`Stage`] here would hide that behind a parameter.
    pub fn violate_at(
        &mut self,
        stage: Stage,
        code: impl Into<String>,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        match stage {
            Stage::Write => self.violate_at_write(code, subject, detail),
            Stage::Publish => self.violate(code, subject, detail),
        }
    }

    /// The write-judgeable part of this report, or `None` when there is none.
    ///
    /// Warnings are dropped rather than carried: an advisory never blocks, and a
    /// write refusal that shipped one alongside its violations would suggest it
    /// might have. The publish pre-check keeps the whole report, this subset
    /// included — this is an earlier refusal of a part, never a replacement.
    #[must_use]
    pub fn write_stage_only(&self) -> Option<Self> {
        let violations: Vec<Violation> = self
            .violations
            .iter()
            .filter(|v| v.stage == Stage::Write)
            .cloned()
            .collect();
        if violations.is_empty() {
            return None;
        }
        Some(Self {
            violations,
            warnings: Vec::new(),
        })
    }

    /// Record an advisory finding.
    pub fn warn(
        &mut self,
        code: impl Into<String>,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.warnings.push(Advisory {
            code: code.into(),
            subject: subject.into(),
            detail: detail.into(),
        });
    }
}

impl fmt::Display for ValidationReport {
    /// Renders the blocking count — the detail belongs in the response
    /// envelope, not in a log line that would repeat it per rule.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.violations.len())
    }
}

/// One registered rule of the aggregate pipeline.
///
/// A rule reads the candidate state it is given and appends its findings; it
/// never short-circuits the run and never mutates the subject. `code_prefix` is
/// declarative: it lets the pipeline report which slice owns a failing rule
/// without the rule having to name itself in every violation.
pub trait ValidationRule<S>: Send + Sync {
    /// Stable rule name for diagnostics and ownership attribution.
    fn name(&self) -> &'static str;

    /// Evaluate the rule against `subject`, appending to `report`.
    fn evaluate(&self, subject: &S, report: &mut ValidationReport);
}

/// The aggregate pipeline: an ordered set of rules over one subject type.
///
/// Rules run in registration order and every one of them runs — the report is
/// the point. A pipeline with no rules publishes everything, which is why the
/// Foundation registers the money and rounding rules itself rather than leaving
/// the base set to whichever slice happens to load first.
#[domain_model]
pub struct ValidationPipeline<S> {
    rules: Vec<Box<dyn ValidationRule<S>>>,
}

impl<S> ValidationPipeline<S> {
    /// An empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Register a rule. Order is preserved and is the order findings appear in.
    #[must_use]
    pub fn with_rule(mut self, rule: Box<dyn ValidationRule<S>>) -> Self {
        self.rules.push(rule);
        self
    }

    /// The registered rule names, in order.
    #[must_use]
    pub fn rule_names(&self) -> Vec<&'static str> {
        self.rules.iter().map(|rule| rule.name()).collect()
    }

    /// Run every rule and return the aggregate report.
    #[must_use]
    pub fn run(&self, subject: &S) -> ValidationReport {
        let mut report = ValidationReport::default();
        for rule in &self.rules {
            rule.evaluate(subject, &mut report);
        }
        report
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
