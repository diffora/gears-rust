//! @cpt-dod:cpt-cf-bss-products-dod-derived-usage-type-rules:p1
//! Derived usage types: the create and new-version rules (P-D-229, P-D-231).
//!
//! A derived usage type is a tenant's catalog data: a stable `id`, a `code` unique in the tenant, a
//! `name` set at the create, and versions 1, 2, … each holding one immutable declaration. The rules
//! of a declaration are the SDK's ([`derived::validate`]); this module turns its typed refusal into
//! the door's 400 `DERIVED_DECLARATION_INVALID`, which names the rule. A version stores the SHA-256
//! of the SDK's canonical bytes, taken here through `aws-lc-rs` (lint DE0708), and nothing ever
//! recomputes it: the stored digest is what pricing pins (P-D-229 decision 3).
//!
//! Every input names a raw GTS usage type, and each is resolved through the [`UsageTypeCatalog`]
//! port as the caller, as a usage SKU's publish is (P-D-184, P-D-207): an unresolved input is 400
//! `USAGE_TYPE_UNRESOLVED`, an unreachable or unconfigured catalog 503 `USAGE_TYPE_UNAVAILABLE`, and
//! a catalog that refuses the caller 403 `USAGE_TYPE_FORBIDDEN`. A derived input is refused before
//! the catalog is asked: [`derived::validate`] refuses the `products.derived/` prefix.
use crate::domain::caps;
use crate::domain::error::DomainError;
use crate::domain::recognized::UsageTypeAnswer;
use crate::domain::validation::ValidationReport;
use aws_lc_rs::digest::{SHA256, digest as sha256};
use bss_products_sdk::derived::{self, DeclarationError, DerivedUsageDeclaration, MeterId};
use bss_products_sdk::usage_types::UsageTypeCatalog;
use time::OffsetDateTime;
use toolkit_macros::domain_model;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The audit `subject_kind` of a derived usage type's acts.
pub const SUBJECT_KIND: &str = "derived_usage_type";
/// The audit action of a create (version 1).
pub const ACTION_CREATE: &str = "derived_usage_type.create";
/// The audit action of a new version (2 and later).
pub const ACTION_VERSION: &str = "derived_usage_type.version";
/// A declaration the shape parse or the SDK refused: 400, naming the rule.
pub const DECLARATION_INVALID: &str = "DERIVED_DECLARATION_INVALID";
/// A second type with the tenant's code: 409.
pub const CODE_TAKEN: &str = "DERIVED_CODE_TAKEN";
/// The head of a version's `accrual_policy_version`: `derived-v1:<digest hex>`.
pub const ACCRUAL_POLICY_PREFIX: &str = "derived-v1:";

// The SDK's caps are the SKU's (Run 1's note): a derived output unit is the selling SKU's unit, and
// an input ref is a usage-type ref.
const _: () = assert!(derived::UNIT_MAX_CHARS == caps::LABEL_MAX_CHARS);
const _: () = assert!(derived::INPUT_REF_MAX_CHARS == caps::USAGE_TYPE_REF_MAX_CHARS);
const _: () = assert!(derived::CODE_MAX_CHARS == caps::CODE_MAX_CHARS);

/// A stored derived usage type.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedUsageType {
    pub tenant_id: Uuid,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
}

/// A stored version of a derived usage type.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedUsageTypeVersion {
    pub tenant_id: Uuid,
    pub type_id: Uuid,
    pub version: u32,
    /// The declaration as the doors serve it (`dto::DerivedDeclarationDto`).
    pub declaration_json: serde_json::Value,
    /// The stored SHA-256 of the canonical bytes, 64 lowercase hex digits.
    pub digest: String,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
}

impl DerivedUsageTypeVersion {
    /// `derived-v1:<stored digest>`: what a pricing usage policy names as its accrual (decision 5).
    #[must_use]
    pub fn accrual_policy_version(&self) -> String {
        format!("{ACCRUAL_POLICY_PREFIX}{}", self.digest)
    }
}

/// What a create writes: the type and its version 1.
#[domain_model]
#[derive(Debug, Clone)]
pub struct NewDerivedType {
    pub code: String,
    pub name: String,
}

/// What a version insert writes.
#[domain_model]
#[derive(Debug, Clone)]
pub struct NewDerivedVersion {
    pub type_id: Uuid,
    pub version: u32,
    pub declaration_json: serde_json::Value,
    pub digest: String,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
}

/// The identity rules of a create: a code the meter id can carry, and a name.
#[must_use]
pub fn check_identity(new: &NewDerivedType) -> ValidationReport {
    let mut report = ValidationReport::new();
    if caps::over(&new.code, caps::CODE_MAX_CHARS) {
        caps::check(&mut report, "code", Some(&new.code), caps::CODE_MAX_CHARS);
    } else if MeterId::new(new.code.as_str(), 1).is_err() {
        report.violate("VALIDATION", "code", "code is ^[a-z0-9][a-z0-9._-]{0,63}$");
    }
    if new.name.trim().is_empty() {
        report.violate("VALIDATION", "name", "name must not be blank");
    }
    caps::check(&mut report, "name", Some(&new.name), caps::NAME_MAX_CHARS);
    report
}

/// The rule a [`DeclarationError`] names, as the 400 carries it: one token per SDK variant.
#[must_use]
pub const fn rule(error: &DeclarationError) -> &'static str {
    match error {
        DeclarationError::EmptyUnit { .. } => "empty_unit",
        DeclarationError::UnitTooLong { .. } => "unit_too_long",
        DeclarationError::ScaleTooLarge { .. } => "scale_too_large",
        DeclarationError::TooFewInputs { .. } => "too_few_inputs",
        DeclarationError::InvalidInputName { .. } => "invalid_input_name",
        DeclarationError::DuplicateInput { .. } => "duplicate_input",
        DeclarationError::EmptyInputRef { .. } => "empty_input_ref",
        DeclarationError::InputRefTooLong { .. } => "input_ref_too_long",
        DeclarationError::DerivedInput { .. } => "derived_input",
        DeclarationError::HoldMissing { .. } => "hold_missing",
        DeclarationError::HoldNotAllowed { .. } => "hold_not_allowed",
        DeclarationError::HoldOutOfRange { .. } => "hold_out_of_range",
        DeclarationError::UnknownInput { .. } => "unknown_input",
        DeclarationError::UnusedInput { .. } => "unused_input",
        DeclarationError::DivisionByZero => "division_by_zero",
        DeclarationError::TooFewOperands { .. } => "too_few_operands",
        DeclarationError::TooDeep => "too_deep",
        DeclarationError::TooManyNodes => "too_many_nodes",
    }
}

/// 400 `DERIVED_DECLARATION_INVALID` on `declaration`, its detail led by the rule: `<rule>: <why>`.
#[must_use]
pub fn declaration_invalid(rule: &str, detail: impl std::fmt::Display) -> DomainError {
    let mut report = ValidationReport::new();
    report.violate(
        DECLARATION_INVALID,
        "declaration",
        format!("{rule}: {detail}"),
    );
    DomainError::Validation(report)
}

/// The SDK's rules, as the door answers them.
///
/// # Errors
/// [`declaration_invalid`] naming the first rule the SDK refused.
pub fn validate(declaration: &DerivedUsageDeclaration) -> Result<(), DomainError> {
    derived::validate(declaration).map_err(|e| declaration_invalid(rule(&e), &e))
}

/// The SHA-256 of [`derived::canonical_bytes`], as 64 lowercase hex digits: what a version stores.
#[must_use]
pub fn digest_hex(declaration: &DerivedUsageDeclaration) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    sha256(&SHA256, &derived::canonical_bytes(declaration))
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            hex.push(char::from(DIGITS[usize::from(b >> 4)]));
            hex.push(char::from(DIGITS[usize::from(b & 0x0f)]));
            hex
        })
}

/// Resolve every input through the catalog, as the caller, in input order; each input is asked
/// once. A refusal of the caller answers 403 and an outage 503, whichever input met it; otherwise
/// every unresolved input is one 400 `USAGE_TYPE_UNRESOLVED` on its ref (P-D-202).
///
/// # Errors
/// [`DomainError::UsageTypeForbidden`], [`DomainError::UsageTypeUnavailable`], or
/// [`DomainError::Validation`] with one violation per unresolved input.
pub async fn resolve_inputs(
    catalog: &dyn UsageTypeCatalog,
    ctx: &SecurityContext,
    declaration: &DerivedUsageDeclaration,
) -> Result<(), DomainError> {
    let mut unresolved = ValidationReport::new();
    let (mut forbidden, mut unavailable) = (None, None);
    for input in &declaration.inputs {
        match catalog.resolve(ctx, &input.usage_type_ref).await {
            UsageTypeAnswer::Resolved(_) => {}
            UsageTypeAnswer::Unresolved => unresolved.violate(
                "USAGE_TYPE_UNRESOLVED",
                format!("declaration.inputs.{}.usage_type_ref", input.name),
                format!(
                    "the usage type catalog does not know {}",
                    input.usage_type_ref
                ),
            ),
            UsageTypeAnswer::Forbidden => {
                forbidden.get_or_insert_with(|| input.usage_type_ref.clone());
            }
            UsageTypeAnswer::Unavailable => {
                unavailable.get_or_insert_with(|| input.usage_type_ref.clone());
            }
        }
    }
    if let Some(reference) = forbidden {
        return Err(DomainError::UsageTypeForbidden(reference));
    }
    if let Some(reference) = unavailable {
        return Err(DomainError::UsageTypeUnavailable(reference));
    }
    if unresolved.is_empty() {
        Ok(())
    } else {
        Err(DomainError::Validation(unresolved))
    }
}

#[cfg(test)]
#[path = "derived_tests.rs"]
mod derived_tests;
