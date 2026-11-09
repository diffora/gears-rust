//! Tenant-name validation against the registered tenant resource schema.
//!
//! Fixed user create/patch commands use the SDK DTO validators instead. This
//! existing tenant-name path retains its caller/platform read split and DB-backed
//! missing-schema fallback; extensible metadata uses the native GTS payload compiler.

use std::collections::BTreeMap;

use serde_json::Value;
use toolkit_canonical_errors::CanonicalError;
use toolkit_gts::gts_id;
use types_registry_sdk::{
    EntityField, FieldSelection, GtsTypeId, LifecycleStatus, Projection, ReadOptions,
};

use crate::domain::error::DomainError;
use crate::domain::types_registry_reader::TypesRegistryReader;

pub(crate) const TENANT_TYPE_ID: &str = gts_id!("cf.core.am.tenant.v1~");

/// Validate a tenant `name` against the `name` sub-schema of the
/// registered `gts.cf.core.am.tenant.v1~` (`minLength: 1, maxLength:
/// 255`).
///
/// `reader` names the caller: the request's context for create / update,
/// the platform API for the bootstrap saga's root name only.
///
/// # Errors
///
/// See gear-level docs.
pub async fn validate_tenant_name_via_gts(
    name: &str,
    reader: TypesRegistryReader<'_>,
) -> Result<(), DomainError> {
    let Some(properties) = lookup_effective_properties(TENANT_TYPE_ID, reader).await? else {
        return Ok(());
    };
    validate_property_value(
        TENANT_TYPE_ID,
        "name",
        &Value::String(name.to_owned()),
        &properties,
    )
}

/// Fetch the effective property map for `type_id`, returning `None`
/// when the schema is not registered (so callers can short-circuit
/// to AM-side guards). See gear-level docs for the failure-mode
/// contract on other registry errors.
async fn lookup_effective_properties(
    type_id: &'static str,
    reader: TypesRegistryReader<'_>,
) -> Result<Option<BTreeMap<String, Vec<Value>>>, DomainError> {
    let projection = Projection::Select(FieldSelection::with(&[EntityField::ResolvedSchema]));
    match reader
        .get_type_schema(
            &GtsTypeId::new(type_id),
            ReadOptions {
                projection,
                ..ReadOptions::default()
            },
        )
        .await
    {
        // A deleted schema is no longer registered for new payloads.
        Ok(schema) if schema.lifecycle_status != LifecycleStatus::Active => Ok(None),
        Ok(schema) => {
            let resolved = schema.resolved_schema.ok_or_else(|| {
                DomainError::internal(format!(
                    "types-registry answered `{type_id}` without the selected resolved_schema"
                ))
            })?;
            Ok(Some(effective_properties(&resolved)))
        }
        Err(CanonicalError::NotFound { .. }) => Ok(None),
        Err(err) => Err(reader.failure(
            err,
            format_args!("GTS type schema lookup failed for `{type_id}`"),
        )),
    }
}

/// Every sub-schema a registry-resolved schema declares for each property.
/// References are inlined at admission, so the `allOf` members carry the
/// inherited declarations. The branches of a schema are conjunctive: a
/// property declared at several places must satisfy all of them, so none
/// overrides another.
fn effective_properties(schema: &Value) -> BTreeMap<String, Vec<Value>> {
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    collect_properties(schema, &mut out);
    out
}

fn collect_properties(schema: &Value, out: &mut BTreeMap<String, Vec<Value>>) {
    if let Some(own) = schema.get("properties").and_then(Value::as_object) {
        for (key, value) in own {
            out.entry(key.clone()).or_default().push(value.clone());
        }
    }
    if let Some(members) = schema.get("allOf").and_then(Value::as_array) {
        for member in members {
            collect_properties(member, out);
        }
    }
}

/// Compile the property sub-schema for `field_name` and validate
/// `field_value` against it.
///
/// A field that has no entry in the schema's `properties` map is
/// treated as **catalog drift**, not as "this field is unconstrained".
/// Every caller passes a hardcoded field name (`name`, `username`,
/// `email`, ...) that AM expects the published schema to declare;
/// a missing entry means the deployed schema is stale or
/// misregistered relative to the AM build. Surfacing this as
/// `Internal` (HTTP 500) makes the catalog-vs-code skew visible to
/// operators instead of silently disabling validation for the
/// drifted field and persisting / forwarding out-of-contract data.
fn validate_property_value(
    type_id: &'static str,
    field_name: &str,
    field_value: &Value,
    properties: &BTreeMap<String, Vec<Value>>,
) -> Result<(), DomainError> {
    let Some(field_schemas) = properties.get(field_name) else {
        return Err(DomainError::Internal {
            diagnostic: format!(
                "GTS schema `{type_id}` does not declare property `{field_name}` (catalog drift: \
                 deployed schema is stale or misregistered relative to the AM build — the field \
                 is hardcoded by the AM service layer and MUST be present in the published \
                 schema's `properties` map)"
            ),
            cause: None,
        });
    };
    let field_schema = match field_schemas.as_slice() {
        [single] => std::borrow::Cow::Borrowed(single),
        several => std::borrow::Cow::Owned(serde_json::json!({ "allOf": several })),
    };
    let validator =
        jsonschema::validator_for(&field_schema).map_err(|err| DomainError::Internal {
            diagnostic: format!(
                "GTS schema `{type_id}` property `{field_name}` is not a valid JSON Schema \
             (catalog drift): {err}"
            ),
            cause: None,
        })?;
    let errors: Vec<String> = validator
        .iter_errors(field_value)
        .map(|e| e.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(DomainError::Validation {
            detail: format!(
                "field `{field_name}` violates `{type_id}` schema: {}",
                errors.join("; ")
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "gts_validation_tests.rs"]
mod gts_validation_tests;
