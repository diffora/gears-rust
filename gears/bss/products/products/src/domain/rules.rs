//! The Foundation's own registered rules, and the candidate they judge.
//!
//! Registration is compile-time code: a feature ships its validators with its
//! handler, and there is no runtime registry to fall out of step with the
//! handler set. The Foundation registers the shape and identity rules itself
//! rather than leaving the base set to whichever capability feature loads
//! first.

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::containment::ResolvedScope;
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

/// The subject a `-> published` transition presents to the registered
/// validators: the head as it stands, plus the facts a rule cannot read for
/// itself.
///
/// [`ValidationRule::evaluate`] is synchronous, so a rule whose operand lives
/// in another table cannot fetch it — the door reads the fact and the subject
/// carries it. That is why this type exists beside
/// [`CreateEntityCandidate`] rather than extending it: a create has no
/// assignments to read, and a field that is meaningless at one door is a
/// field the next reader has to reason about.
#[derive(Clone, Debug)]
pub struct PublishedTransitionSubject {
    /// Whether the Product holds a `primary` category assignment
    /// (`products_product_category`, the single source of truth).
    pub has_primary_category: bool,
}

/// A Product reaching `published` must carry its primary category
/// (`inst-tx-primary-at-publish`).
///
/// **Optional at draft, required at publish** — the PRD's own words, and the
/// reason this rule lives in the publish edge's pipeline and not in the
/// Foundation's shared re-validation one: a save on a draft carrying no
/// primary is legal, and a rule registered in the shared pipeline would
/// refuse it. The *at-most-one* half is not here at all, being a partial
/// unique index on the assignment table rather than a rule.
///
/// The code is **this slice's**, declared by `design/02` §3.3 under 01 §3.3's
/// code-to-declaring-slice rule (**P-D-36**).
pub struct PrimaryCategoryRequired;

impl PrimaryCategoryRequired {
    /// The refusal code, 422 architectural (`design/02` §3.3).
    pub const CODE: &'static str = "PRIMARY_CATEGORY_REQUIRED";
}

impl ValidationRule<PublishedTransitionSubject> for PrimaryCategoryRequired {
    fn name(&self) -> &'static str {
        "inst-tx-primary-at-publish"
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &PublishedTransitionSubject, report: &mut ValidationReport) {
        if !subject.has_primary_category {
            report.violate(
                Self::CODE,
                "categories",
                "a Product must carry a primary category assignment to be published",
            );
        }
    }
}

/// The candidate the SKU publish door's re-validation re-run judges: the head
/// **as it now stands**, reduced to the columns the re-run's rules read.
///
/// @cpt-cf-bss-products-dod-publish-door
///
/// # Why this type exists rather than the repository record
///
/// The two rules below judged `infra::storage::repo::SkuRecord` directly and
/// were declared in `api::rest::skus` beside the door that ran them. Both
/// halves of that were wrong for the same reason, and the second is the
/// load-bearing one. `inst-fd-publish-revalidate` re-runs **the pipeline**,
/// and the point of writing these as rules rather than as `if`s in the door
/// is that slices 04 and 05 register their `-> published` validators beside
/// them; a rule whose subject is a **repository DTO** cannot be registered by
/// either without that slice taking a dependency on `infra::storage::repo`,
/// which inverts the gear's own layering. The subject a domain rule judges has
/// to be a domain value, so this is one — built by the door from the record
/// the way [`CreateEntityCandidate`] is built from a payload, and the way
/// `api::rest::products::publish_candidate` already builds one on the Product
/// side.
///
/// It carries three columns and not the whole head on purpose: a subject
/// wider than its rules read invites a fourth rule to reach for a column the
/// re-run was never scoped over, and `internal_revision`, `lifecycle_state`
/// and `published_version` are decided by the door's own precondition,
/// terminality and edge steps rather than by a registered rule.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishRevalidationSubject {
    /// The head's `sku_code`, as stored.
    pub sku_code: String,
    /// The head's stored region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The head's stored brand value set. Same reading.
    pub brand_scope: String,
}

/// The `sku_code` a head must still carry to be publishable.
///
/// @cpt-cf-bss-products-dod-publish-door
///
/// A [`ValidationRule`] rather than an `if` in the door, because
/// `inst-fd-publish-revalidate` re-runs **the pipeline**, and a check written
/// as an `if` is one slices 04 and 05 cannot register beside their own.
pub struct SkuCodeStillPresent;

impl ValidationRule<PublishRevalidationSubject> for SkuCodeStillPresent {
    fn name(&self) -> &'static str {
        "inst-fd-publish-revalidate/sku_code"
    }

    fn phase(&self) -> Phase {
        Phase::Shape
    }

    fn evaluate(&self, subject: &PublishRevalidationSubject, report: &mut ValidationReport) {
        if subject.sku_code.trim().is_empty() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "sku_code",
                "sku_code is blank, so this entity is no longer publishable",
            );
        }
    }
}

/// The two stored scope columns must still parse under
/// [`ResolvedScope::parse`]'s own rule.
///
/// @cpt-cf-bss-products-dod-publish-door
///
/// [`Phase::Identity`] because §4.2 files containment and reservation there,
/// and this is the operand the containment rule reads.
pub struct SkuScopeColumnsStillParse;

impl ValidationRule<PublishRevalidationSubject> for SkuScopeColumnsStillParse {
    fn name(&self) -> &'static str {
        "inst-fd-publish-revalidate/scope-columns"
    }

    fn phase(&self) -> Phase {
        Phase::Identity
    }

    fn evaluate(&self, subject: &PublishRevalidationSubject, report: &mut ValidationReport) {
        if ResolvedScope::parse(&subject.region_scope).is_err() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "region_scope",
                "region_scope contains an empty value between separators",
            );
        }
        if ResolvedScope::parse(&subject.brand_scope).is_err() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "brand_scope",
                "brand_scope contains an empty value between separators",
            );
        }
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;
