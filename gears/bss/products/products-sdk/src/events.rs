//! The event schema roster — every registry event as a **versioned
//! artifact** (`inst-rc-versioning`, `PRD` §9.2 `contract-registry-events`:
//! *"versioned schema refs (semver)"*).
//!
//! # What lives here and what does not
//!
//! Each row pairs a payload type token with its semver schema reference,
//! `bss-products.<Token>.v<major.minor.patch>`, the string the outbox
//! envelope's `schema_ref` carries. The **deserializable payload types do not
//! live here** (P-D-130): every typed event derives `Deserialize` in the
//! gear's `infra::broker`, where the compatibility probe runs C2's direction
//! — an old consumer reading a new payload — and this crate stays serde-free
//! by its own module doc. A consumer that needs the shapes reads the schema
//! reference off the envelope and this roster tells it which version it is
//! looking at; a consumer that needs the Rust types takes them from the gear.
//!
//! The gear's `infra::events::SCHEMA_REFS` is held equal to this table by
//! `infra::events_tests`, so a forty-first event registered in the gear fails
//! there until it is versioned here — the roster is the artifact, and the two
//! copies are the pin. Bumping a version is a **breaking** change for the
//! major and additive for the minor, on the ordinary semver reading.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-event-versioning:p1

/// `(payload type token, versioned schema reference)`, one row per event
/// the registry emits. **Forty** at this revision.
pub const SCHEMA_REFS: &[(&str, &str)] = &[
    ("ProductCreated", "bss-products.ProductCreated.v1.0.0"),
    ("SkuCreated", "bss-products.SkuCreated.v1.0.0"),
    ("ProductHeadSaved", "bss-products.ProductHeadSaved.v1.0.0"),
    ("SkuHeadSaved", "bss-products.SkuHeadSaved.v1.0.0"),
    ("ProductPublished", "bss-products.ProductPublished.v1.0.0"),
    ("SkuPublished", "bss-products.SkuPublished.v1.0.0"),
    ("ProductDiscarded", "bss-products.ProductDiscarded.v1.0.0"),
    ("SkuDiscarded", "bss-products.SkuDiscarded.v1.0.0"),
    ("ProductDeprecated", "bss-products.ProductDeprecated.v1.0.0"),
    ("SkuDeprecated", "bss-products.SkuDeprecated.v1.0.0"),
    (
        "RecognizedUnitUpdated",
        "bss-products.RecognizedUnitUpdated.v1.0.0",
    ),
    (
        "RecognizedCodeUpdated",
        "bss-products.RecognizedCodeUpdated.v1.0.0",
    ),
    ("PlanTierUpdated", "bss-products.PlanTierUpdated.v1.0.0"),
    (
        "SkuImmutableFieldCorrected",
        "bss-products.SkuImmutableFieldCorrected.v1.0.0",
    ),
    (
        "SkuCorrectionOverride",
        "bss-products.SkuCorrectionOverride.v1.0.0",
    ),
    (
        "ReferenceProducerSetChanged",
        "bss-products.ReferenceProducerSetChanged.v1.0.0",
    ),
    (
        "CatalogVersionPublished",
        "bss-products.CatalogVersionPublished.v1.0.0",
    ),
    (
        "FreezeForceCompleted",
        "bss-products.FreezeForceCompleted.v1.0.0",
    ),
    (
        "FreezeParticipantSetChanged",
        "bss-products.FreezeParticipantSetChanged.v1.0.0",
    ),
    (
        "SkuCompositionCleared",
        "bss-products.SkuCompositionCleared.v1.0.0",
    ),
    (
        "CatalogBulkOperationCompleted",
        "bss-products.CatalogBulkOperationCompleted.v1.0.0",
    ),
    (
        "ProductUndeprecated",
        "bss-products.ProductUndeprecated.v1.0.0",
    ),
    ("SkuUndeprecated", "bss-products.SkuUndeprecated.v1.0.0"),
    ("SkuRetired", "bss-products.SkuRetired.v1.0.0"),
    ("ProductRetired", "bss-products.ProductRetired.v1.0.0"),
    (
        "SkuRetirementEffective",
        "bss-products.SkuRetirementEffective.v1.0.0",
    ),
    (
        "ProductRetirementEffective",
        "bss-products.ProductRetirementEffective.v1.0.0",
    ),
    ("CategoryCreated", "bss-products.CategoryCreated.v1.0.0"),
    ("CategoryRenamed", "bss-products.CategoryRenamed.v1.0.0"),
    (
        "CategoryReparented",
        "bss-products.CategoryReparented.v1.0.0",
    ),
    ("CategoryRetired", "bss-products.CategoryRetired.v1.0.0"),
    ("CategoryDeleted", "bss-products.CategoryDeleted.v1.0.0"),
    ("MetadataUpdated", "bss-products.MetadataUpdated.v1.0.0"),
    (
        "CategoryDisplayUpdated",
        "bss-products.CategoryDisplayUpdated.v1.0.0",
    ),
    (
        "AttributeDefinitionUpdated",
        "bss-products.AttributeDefinitionUpdated.v1.0.0",
    ),
    ("ActorErased", "bss-products.ActorErased.v1.0.0"),
    (
        "PiiAllowlistChanged",
        "bss-products.PiiAllowlistChanged.v1.0.0",
    ),
    ("ApprovalDecided", "bss-products.ApprovalDecided.v1.0.0"),
    (
        "BreakGlassElevated",
        "bss-products.BreakGlassElevated.v1.0.0",
    ),
    ("BreakGlassExpired", "bss-products.BreakGlassExpired.v1.0.0"),
];

/// The schema reference a payload type token is versioned under; `None` for
/// a token the registry does not emit.
#[must_use]
pub fn schema_ref(payload_type: &str) -> Option<&'static str> {
    SCHEMA_REFS
        .iter()
        .find(|(token, _)| *token == payload_type)
        .map(|(_, schema)| *schema)
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_REFS, schema_ref};

    #[test]
    fn every_reference_is_the_tokens_own_semver_ref() {
        assert_eq!(SCHEMA_REFS.len(), 40);
        let mut tokens = std::collections::BTreeSet::new();
        for (token, schema) in SCHEMA_REFS {
            assert!(tokens.insert(*token), "{token} is listed once");
            let expected_prefix = format!("bss-products.{token}.v");
            assert!(
                schema.starts_with(&expected_prefix),
                "{schema} names {token}"
            );
            let version = &schema[expected_prefix.len()..];
            assert_eq!(version.split('.').count(), 3, "{schema} is semver");
            assert!(
                version.split('.').all(|p| p.parse::<u32>().is_ok()),
                "{schema} is numeric"
            );
            assert_eq!(schema_ref(token), Some(*schema));
        }
        assert_eq!(schema_ref("NotAnEvent"), None);
    }
}
