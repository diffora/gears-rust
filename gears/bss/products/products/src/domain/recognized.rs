//! Usage-type binding and meter declaration checks.

use crate::domain::error::DomainError;

pub use bss_products_sdk::usage_types::{UsageTypeAnswer, UsageTypeBinding};

/// What the catalog said a `usageTypeRef` is bound to, in the stored form
/// frozen beside the version row at publish (`dod-binding-snapshot`,
/// **P-D-134** row 6, **P-D-146**): one JSON object, keys in alphabetical
/// order, the metadata keys sorted — so two publishes of the same binding
/// store the same bytes.
///
/// **Provenance, not content**: the snapshot lives in its own nullable column
/// on `products_entity_version`, outside the digested rendering, so
/// `DIGEST_VERSION` does not move with it and a re-verification of the digest
/// never reads it. The three fields are the three the definition of done
/// names; the catalog's own types are flattened to strings at the port so the
/// domain owes the collector SDK nothing.
///
/// This paragraph moved here with the types (2026-09-22): it argues about the
/// **snapshot**, which is this function's business, not about the shape of the
/// binding, which is now the port's.
///
/// A free function rather than an inherent method, because the type is the
/// SDK's now and this rendering is the **domain's** business: the snapshot is
/// what `binding_snapshot` freezes and what a re-verification reads, and the
/// port has no opinion about it.
#[must_use]
pub fn binding_snapshot_json(binding: &UsageTypeBinding) -> String {
    let mut fields = binding.metadata_fields.clone();
    fields.sort();
    serde_json::json!({
        "gts_id": binding.gts_id,
        "kind": binding.kind,
        "metadata_fields": fields,
    })
    .to_string()
}

/// Map a pre-transaction resolve onto the publish refusal
/// (**P-D-121** row 19). The validators phase receives the answer and
/// never calls out. A resolved answer hands back the binding the publish
/// freezes beside the version row (`dod-binding-snapshot`).
///
/// # Errors
///
/// [`DomainError::UsageTypeUnresolved`] or [`DomainError::UsageTypeUnavailable`].
pub fn judge_usage_type(
    answer: UsageTypeAnswer,
    usage_type_ref: &str,
) -> Result<UsageTypeBinding, DomainError> {
    match answer {
        UsageTypeAnswer::Resolved(binding) => Ok(binding),
        UsageTypeAnswer::Unresolved => Err(DomainError::UsageTypeUnresolved(format!(
            "usageTypeRef `{usage_type_ref}` did not resolve in the collector"
        ))),
        UsageTypeAnswer::Unavailable => Err(DomainError::UsageTypeUnavailable(format!(
            "the usage-type collector did not answer for `{usage_type_ref}`"
        ))),
    }
}

/// The atomic-pair rule (`inst-mt-atomic-pair`, `dod-meter-atomic`): the
/// resulting row carries `metering_unit` and `usage_type_ref` together or
/// not at all. The paired `CHECK` refuses the same shape at the physical
/// layer; this is the door's half, with the code the taxonomy names.
///
/// # Errors
///
/// [`DomainError::MeterDeclarationIncomplete`].
pub fn meter_pair_complete(
    metering_unit: Option<&str>,
    usage_type_ref: Option<&str>,
) -> Result<(), DomainError> {
    if metering_unit.is_some() == usage_type_ref.is_some() {
        return Ok(());
    }
    let (present, absent) = if metering_unit.is_some() {
        ("metering_unit", "usage_type_ref")
    } else {
        ("usage_type_ref", "metering_unit")
    };
    Err(DomainError::MeterDeclarationIncomplete(format!(
        "a MeterDeclaration is atomic: {present} arrived without {absent}, and the pair travels \
         together or not at all"
    )))
}

#[cfg(test)]
#[path = "recognized_tests.rs"]
mod recognized_tests;
