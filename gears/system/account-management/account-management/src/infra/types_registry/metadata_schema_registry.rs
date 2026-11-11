//! Production [`MetadataSchemaRegistry`] over `types_registry_sdk::TypesRegistryApi`.
//!
//! Check existence before database access; an unknown schema yields
//! [`DomainError::MetadataEntryNotFound`]. A deleted schema keeps its exact
//! identity as a tombstone: entries stored under it stay readable and removable
//! (existence gate and reverse lookups), while its policy and value validation,
//! which serve new values and the inheritance walk, treat it as absent. Read chain-merged `inheritance_policy`
//! from effective traits: only `"inherit"` selects [`InheritancePolicy::Inherit`];
//! missing, null or other values select [`InheritancePolicy::OverrideOnly`].
//! Reverse lookup returns the schema's chained `type_id` by UUID.
//! Value validation selects authored content and the resolved schema, then uses
//! the SDK's native GTS payload compiler on the `{"payload": value}` envelope.
//!
//! Missing selected documents are internal contract errors. The adapter keeps
//! no cache; short-lived caching belongs to the SDK client.

use async_trait::async_trait;
use gts::{GtsId, GtsTypeId};
use serde_json::Value;
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use types_registry_sdk::{
    EntityField, FieldSelection, LifecycleStatus, Projection, ReadOptions, TypeSchema,
    TypesRegistryApi, TypesRegistryApiExt,
};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::metadata::registry::{InheritancePolicy, MetadataSchemaRegistry};

/// Top-level key in the effective trait map carrying the inheritance
/// policy enum value. Defined on the `gts.cf.core.am.tenant_metadata.v1~`
/// envelope's `x-gts-traits-schema` (see
/// `account_management_sdk::gts_envelopes::TenantMetadataTraits`, the
/// Rust source of truth the macro emits the trait schema from).
const INHERITANCE_POLICY_TRAIT: &str = "inheritance_policy";

/// Wire token for the `Inherit` policy. Anything else (missing key,
/// `null`, non-string value, unknown string) collapses to the
/// documented default ([`InheritancePolicy::OverrideOnly`]) per
/// FEATURE §3.1.
const INHERIT_POLICY_TOKEN: &str = "inherit";
const OVERRIDE_POLICY_TOKEN: &str = "override_only";

/// Production [`MetadataSchemaRegistry`] backed by the GTS Types
/// Registry. Shared by every metadata request; each read is made for the
/// caller passed with it.
pub struct GtsMetadataSchemaRegistry {
    registry: Arc<dyn TypesRegistryApi>,
}

impl GtsMetadataSchemaRegistry {
    /// Construct a new registry adapter around the tenant registry
    /// client resolved from `ClientHub`.
    #[must_use]
    pub fn new(registry: Arc<dyn TypesRegistryApi>) -> Self {
        Self { registry }
    }
}

/// Map a `CanonicalError` onto the appropriate `DomainError` for
/// the schema-registry seam:
///
/// * `CanonicalError::NotFound` → [`DomainError::MetadataEntryNotFound`]
///   (HTTP 404 with the unified `resource_type =
///   gts.cf.core.am.tenant_metadata.v1~`).
/// * anything else → [`DomainError::caller_registry_failure`]: the
///   caller's 401 / 403 for a refusal of its credential,
///   `ServiceUnavailable` for an outage, `Internal` otherwise.
fn map_registry_err(err: CanonicalError, schema_token: &str) -> DomainError {
    match err {
        CanonicalError::NotFound { .. } => not_registered(schema_token),
        other => DomainError::caller_registry_failure(other, "types-registry"),
    }
}

/// One Type Schema by identifier under `projection`, active or deleted.
async fn read_schema(
    registry: &dyn TypesRegistryApi,
    ctx: &SecurityContext,
    type_id: &GtsTypeId,
    projection: Projection,
) -> Result<TypeSchema, DomainError> {
    registry
        .get_type_schema(
            ctx,
            type_id,
            ReadOptions {
                projection,
                ..ReadOptions::default()
            },
        )
        .await
        .map_err(|err| map_registry_err(err, type_id.as_ref()))
}

/// [`read_schema`] for a new value or the inheritance walk: a deleted schema
/// is as absent as an unknown one.
async fn read_active_schema(
    registry: &dyn TypesRegistryApi,
    ctx: &SecurityContext,
    type_id: &GtsTypeId,
    projection: Projection,
) -> Result<TypeSchema, DomainError> {
    let schema = read_schema(registry, ctx, type_id, projection).await?;
    if schema.lifecycle_status != LifecycleStatus::Active {
        return Err(not_registered(type_id.as_ref()));
    }
    Ok(schema)
}

fn not_registered(schema_token: &str) -> DomainError {
    DomainError::MetadataEntryNotFound {
        detail: format!("schema {schema_token} is not registered in the types registry"),
        entry: schema_token.to_owned(),
    }
}

fn missing_document(type_id: &GtsTypeId, field: &str) -> DomainError {
    DomainError::internal(format!(
        "types-registry answered metadata schema `{type_id}` without the selected {field}"
    ))
}

/// The UUID round-trip guard: re-derive the UUID from the answered
/// `type_id` and confirm it matches the requested one.
fn round_tripped(schema: &TypeSchema, schema_uuid: Uuid) -> Result<GtsTypeId, DomainError> {
    // Reject UUID/type-ID mismatches as Internal to prevent incorrect list hydration.
    // MetadataService::upsert_metadata already validates the AM chain prefix via
    // ParsedTypeId::parse; this adapter only checks the registry's UUID round-trip.
    let raw = schema.type_id.as_ref();
    let answered = GtsId::try_new(raw).map_err(|err| {
        DomainError::internal(format!(
            "types-registry returned schema {raw} for uuid {schema_uuid}, which is not a GTS id: \
             {err}"
        ))
    })?;
    if answered.to_uuid() != schema_uuid {
        return Err(DomainError::Internal {
            diagnostic: format!(
                "types-registry returned schema {raw} for uuid {schema_uuid} but the AM-side \
                 UUIDv5 derivation does not round-trip; possible SDK bug or schema renaming \
                 mid-flight"
            ),
            cause: None,
        });
    }
    Ok(GtsTypeId::new(raw))
}

#[async_trait]
impl MetadataSchemaRegistry for GtsMetadataSchemaRegistry {
    async fn resolve_inheritance_policy(
        &self,
        ctx: &SecurityContext,
        type_id: &GtsTypeId,
    ) -> Result<InheritancePolicy, DomainError> {
        let schema = read_active_schema(
            self.registry.as_ref(),
            ctx,
            type_id,
            Projection::Select(FieldSelection::with(&[EntityField::EffectiveTraits])),
        )
        .await?;

        // The registry materializes `effective_traits` walking self →
        // ancestors with rightmost-wins semantics, then layering
        // schema-declared `default` values from every
        // `x-gts-traits-schema` block in the chain. The base
        // envelope (`gts.cf.core.am.tenant_metadata.v1~`) declares
        // `inheritance_policy: { default: "override_only" }` so a
        // properly-derived schema always carries a string value here.
        let effective = schema
            .effective_traits
            .as_ref()
            .ok_or_else(|| missing_document(type_id, "effective_traits"))?;
        let raw = effective.get(INHERITANCE_POLICY_TRAIT);
        let policy = match raw {
            // Explicit `"inherit"` opt-in.
            Some(v) if v.as_str() == Some(INHERIT_POLICY_TOKEN) => InheritancePolicy::Inherit,
            // Explicit `"override_only"` OR an absent / null trait
            // (FEATURE §3.1 defaults to override-only when the trait
            // is unspecified). Both are valid wire shapes; treat as
            // the documented default silently.
            Some(v) if v.as_str() == Some(OVERRIDE_POLICY_TOKEN) => InheritancePolicy::OverrideOnly,
            None => InheritancePolicy::OverrideOnly,
            Some(v) if v.is_null() => InheritancePolicy::OverrideOnly,
            // Truly unknown shape: typo, future variant, wrong wire
            // type. Warn so the operator sees the drift before users
            // observe unexpected walk-up behaviour, then default to
            // override-only (the safe-default in the walk-up
            // algorithm -- an unrecognised value cannot accidentally
            // promote a schema to inheriting from ancestors).
            Some(other) => {
                tracing::warn!(
                    target: "am.metadata",
                    type_id = %type_id,
                    raw_value = ?other,
                    "unknown inheritance_policy value; defaulting to override_only"
                );
                InheritancePolicy::OverrideOnly
            }
        };
        Ok(policy)
    }

    async fn resolve_ids_by_uuid(
        &self,
        ctx: &SecurityContext,
        schema_uuids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, GtsTypeId>, DomainError> {
        // One batch read per page. A failed read fails the whole lookup;
        // unknown schemas are omitted (page-poisoning guard) and the service
        // layer decides per-row how to surface the miss. A deleted schema
        // still names its stored entries.
        let found = self
            .registry
            .batch_get_type_schemas_by_uuid(ctx, schema_uuids, ReadOptions::default())
            .await
            .map_err(|err| DomainError::caller_registry_failure(err, "types-registry"))?;
        let mut out = std::collections::HashMap::with_capacity(found.len());
        for (uuid, schema) in found {
            if let Some(schema) = schema {
                out.insert(uuid, round_tripped(&schema, uuid)?);
            }
        }
        Ok(out)
    }

    async fn resolve_id_by_uuid(
        &self,
        ctx: &SecurityContext,
        schema_uuid: Uuid,
    ) -> Result<GtsTypeId, DomainError> {
        let token = schema_uuid.to_string();
        let schema = self
            .registry
            .get_type_schema_by_uuid(ctx, schema_uuid, ReadOptions::default())
            .await
            .map_err(|err| map_registry_err(err, &token))?;
        round_tripped(&schema, schema_uuid)
    }

    async fn check_schema_retained(
        &self,
        ctx: &SecurityContext,
        type_id: &GtsTypeId,
    ) -> Result<(), DomainError> {
        read_schema(
            self.registry.as_ref(),
            ctx,
            type_id,
            Projection::Select(FieldSelection::light()),
        )
        .await
        .map(drop)
    }

    async fn validate_value(
        &self,
        ctx: &SecurityContext,
        type_id: &GtsTypeId,
        value: &Value,
    ) -> Result<(), DomainError> {
        // Validate the resolved schema and collect all violations into MetadataValidation.
        // Caller-named schemas missing from the registry return MetadataEntryNotFound
        // (404), rather than a deployment-prerequisite error.
        let schema = read_active_schema(
            self.registry.as_ref(),
            ctx,
            type_id,
            Projection::Select(FieldSelection::with(&[
                EntityField::Content,
                EntityField::ResolvedSchema,
            ])),
        )
        .await?;
        let validator = schema.compile_payload_validator().map_err(|error| {
            DomainError::internal(format!(
                "GTS metadata schema `{type_id}` cannot compile a payload validator \
                 (catalog drift): {error}"
            ))
        })?;
        // An instance of the metadata type is `{"payload": {…}}`, not the bare
        // object (§4.4.1, `TenantMetadataEnvelopeV1`). AM stores the payload content,
        // so the envelope is composed here rather than demanded of callers — the only
        // seam that validates a metadata value as a whole document.
        let enveloped = serde_json::json!({ "payload": value });
        validator
            .validate(&enveloped)
            .map_err(|error| DomainError::MetadataValidation {
                detail: format!("metadata value violates registered schema `{type_id}`: {error}"),
            })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "metadata_schema_registry_tests.rs"]
mod metadata_schema_registry_tests;
