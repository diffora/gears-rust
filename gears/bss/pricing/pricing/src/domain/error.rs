//! `DomainError` — the gear's internal rejection vocabulary.
//!
//! Every variant is mapped to the AIP-193
//! [`toolkit_canonical_errors::CanonicalError`] envelope by the SINGLE
//! authoritative ladder in [`crate::infra::error_mapping`], so a domain
//! rejection is assigned a canonical category in exactly one place and both
//! surfaces — REST and the in-process client — agree by construction.
//!
//! The Foundation owns the problem types listed in `design/01-foundation.md`
//! §3.3; slices **reference** them and never redefine them, which is why they
//! live here rather than beside the surface that raises them. The wire codes
//! (`DUPLICATE_SCOPE_KEY`, `AMOUNT_NEGATIVE`, …) are the machine-readable
//! discriminators a consumer matches after the coarse category; they are the
//! names the design set uses, verbatim.

use toolkit_macros::domain_model;

use crate::domain::validation::ValidationReport;

/// A catalog operation rejection.
///
/// Fail-closed is the rule the taxonomy encodes: there is no variant that means
/// "proceeded with a default". An absent required field is a publish failure
/// ([`DomainError::ValidationFailed`]), never a downstream substitution.
#[domain_model]
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    // -- InvalidArgument (bad request shape / value) --
    /// A request the gear cannot interpret at all — malformed ids, an unknown
    /// enum value, a missing required body field.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    // -- FailedPrecondition (state forbids the operation) --
    /// A published row resolves neither a row-level `rounding_policy_ref` nor a
    /// tenant default. Rounding decides the last minor unit of every charge, so
    /// an unresolved policy fails publish rather than picking one.
    #[error("rounding policy unresolved: {0}")]
    RoundingPolicyUnresolved(String),
    /// An amount carries more precision than the currency's ISO 4217 minor
    /// unit. There is no flat two-decimal rule: JPY takes 0, BHD takes 3.
    #[error("precision exceeds the currency minor unit: {0}")]
    PrecisionExceeded(String),
    /// A negative amount. Typed credit rows are deliberately Future scope, so a
    /// negative price is a mistake, not an unsupported feature.
    #[error("amount must be >= 0: {0}")]
    AmountNegative(String),
    /// A currency code that is not valid ISO 4217.
    #[error("invalid ISO 4217 currency: {0}")]
    CurrencyInvalid(String),
    /// Publish of a `modelKind` (or a non-`sum` aggregation row) that no green
    /// joint fixture gates. The corpus is the conformance contract with Rating;
    /// publishing past a missing fixture would ship a shape no one has agreed
    /// how to evaluate.
    #[error("no green conformance fixture: {0}")]
    FixtureMissing(String),
    /// The state machine forbids the transition — mutating a published row,
    /// re-publishing a retired plan, superseding a grandfathered row.
    #[error("lifecycle transition forbidden: {0}")]
    LifecycleForbidden(String),

    // -- Aborted (conflict; the caller may retry with fresh state) --
    /// Another current row already occupies the canonical scope key.
    #[error("duplicate canonical scope key: {0}")]
    DuplicateScopeKey(String),
    /// The submitted `ETag` / row version is stale: an interactive edit and a
    /// bulk run collided, or the caller is working from a read it did not
    /// refresh. Neither change is silently overwritten.
    #[error("stale version: {0}")]
    StaleVersion(String),
    /// The same idempotency key arrived with a different payload. Never
    /// replayed and never re-executed — the two requests disagree about what
    /// they are.
    #[error("idempotency payload mismatch: {0}")]
    IdempotencyPayloadMismatch(String),

    // -- The aggregate validation envelope --
    /// The fail-closed validation pipeline rejected the publish. Carries the
    /// whole report rather than the first failure: authoring remediates a plan
    /// in one pass, and a truncated report turns one publish into N round
    /// trips.
    #[error("publish validation failed: {0} blocking violation(s)")]
    ValidationFailed(ValidationReport),

    // -- NotFound --
    /// The named subject does not exist, or lies outside the caller's scope —
    /// deliberately the same answer either way, so the surface leaks no
    /// existence. `subject` is the noun (`plan`, `price`, `overlay`), `id` the
    /// reference the caller supplied.
    #[error("{subject} {id} not found")]
    NotFound {
        /// The kind of thing that was not found.
        subject: String,
        /// The identifier the caller asked for.
        id: String,
    },

    // -- Unavailable (fail closed, retry later) --
    /// The `CatalogVersion` registry could not be reached or has no registered
    /// client. Publish stops here: addressability is not optional, and the
    /// registry is the sole incrementer.
    #[error("catalog-version registry unavailable: {0}")]
    CatalogVersionUnavailable(String),
    /// The read model is unavailable. The read path fails closed rather than
    /// serving a stale or partial version.
    #[error("read model unavailable: {0}")]
    ReadModelUnavailable(String),

    // -- Internal --
    /// An infrastructure fault with no domain meaning.
    #[error("internal: {0}")]
    Internal(String),
}
