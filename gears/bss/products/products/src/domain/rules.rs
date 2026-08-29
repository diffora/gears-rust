//! The Foundation's own registered rules, and the candidate they judge.
//!
//! Registration is compile-time code: a feature ships its validators with its
//! handler, and there is no runtime registry to fall out of step with the
//! handler set. The Foundation registers the shape and identity rules itself
//! rather than leaving the base set to whichever capability feature loads
//! first.

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::name;
use crate::domain::validation::{Phase, ValidationReport, ValidationRule};

/// The candidate a create door presents to the pipeline.
///
/// @cpt-cf-bss-products-flow-create-product
/// @cpt-cf-bss-products-flow-define-sku
///
/// It carries the payload as parsed and the normalization the identity phase
/// will key on — computed once, here, rather than by each rule that needs it,
/// so no two rules can disagree about what the name normalizes to.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateEntityCandidate {
    /// The tenant the row will be scoped to.
    pub tenant_id: Uuid,
    /// The brand, a required payload field validated against the caller's
    /// claims before the pipeline opens.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The optional external mapping code.
    pub code: Option<String>,
}

impl CreateEntityCandidate {
    /// The uniqueness operand this candidate would occupy.
    #[must_use]
    pub fn name_normalized(&self) -> String {
        name::normalize(&self.name)
    }
}

/// The name must survive normalization as a non-empty string.
///
/// @cpt-cf-bss-products-fr-create-product
/// @cpt-cf-bss-products-principle-registered-validators
///
/// This is a **shape** rule, not an identity one: it judges the payload alone
/// and never reads the store. The collision check that does read the store is
/// decided under the write by the partial unique index, which is why it is not
/// a rule here — a read-then-act uniqueness check is exactly the race the index
/// exists to lose.
pub struct NameShapeRule;

impl NameShapeRule {
    /// The wire code this rule raises.
    ///
    /// A literal, and it has to be: deriving it by calling
    /// `DomainError::Validation(ValidationReport::new()).code()` is what this
    /// wanted to be, but that value owns a `String` and a value with a
    /// destructor cannot be dropped in a `const` initializer (`E0493`).
    /// `code()` being a `const fn` is not enough on its own.
    ///
    /// So the anti-drift guarantee is a test's rather than the compiler's:
    /// `the_rules_code_constant_cannot_drift_from_domain_errors_own` asserts
    /// this constant against `DomainError::code()`'s own answer, so a rename
    /// on either side reddens. The rule still reads its own constant rather
    /// than a bare literal at the raise site, which is the other half of why
    /// this exists.
    pub const CODE: &'static str = "VALIDATION";
}

impl ValidationRule<CreateEntityCandidate> for NameShapeRule {
    fn name(&self) -> &'static str {
        "inst-fd-name-unique"
    }

    fn phase(&self) -> Phase {
        Phase::Shape
    }

    fn evaluate(&self, subject: &CreateEntityCandidate, report: &mut ValidationReport) {
        if subject.name_normalized().is_empty() {
            report.violate(
                Self::CODE,
                "name",
                "a name must contain at least one non-whitespace character after normalization",
            );
        }
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
