//! GTS (Global Type System) declarations for the credstore SDK.
//!
//! Credstore type/resource ids follow the platform convention
//! `gts.cf.core.credstore.<resource>.v1~`: the trailing `~` terminates the type
//! URI and `.v1` is the schema version. Plugin specs nest under the toolkit base,
//! `gts.cf.toolkit.plugins.plugin.v1~cf.core.credstore.<spec>.v1~`.
//!
//! Each `#[gts_type_schema(...)]`-annotated struct submits its schema to the
//! link-time inventory the types-registry loads at boot. Registration is what
//! makes a resource type authorizable: RBAC validates a role's `target_type`
//! against the registry and the PDP compiles a tenant-scoped grant into an
//! `InTenantSubtree` constraint on `owner_tenant_id`. The same string is the
//! single source of truth for the canonical-error `resource_type` — see
//! [`SECRET_RESOURCE_TYPE`], pinned to the impl crate's `#[resource_error(...)]`
//! marker by a unit test.

use gts_macros::GtsTraitsSchema;
use schemars::JsonSchema;
use toolkit::gts::PluginV1;
use toolkit_gts::gts_type_schema;

/// GTS resource-type id for the credstore **secret** base type — the single
/// source of truth for this string. Mirrored by [`SecretV1`]'s `type_id` and
/// the `#[resource_error(...)]` marker in `infra::sdk_error_mapping` (a unit
/// test there pins that marker to this constant). Enforcement authorizes
/// against the secret's full *concrete* type, not this base: the impl crate's
/// `domain::authz::secret_type_resource(gts_id)` builds the per-operation PEP
/// `ResourceType` from the type resolved out of the types-registry, and the
/// resolver checks a type descends from this base id.
pub const SECRET_RESOURCE_TYPE: &str = "gts.cf.core.credstore.secret.v1~";

#[derive(Default)]
#[gts_type_schema(
    dir_path = "schemas",
    base = PluginV1,
    type_id = "gts.cf.toolkit.plugins.plugin.v1~cf.core.credstore.plugin.v1~",
    description = "CredStore plugin specification",
    properties = "",
)]
pub struct CredStorePluginSpecV1;

/// GTS type-schema for the credstore **secret** resource.
///
/// This is the base of the PEP `ResourceType`s credstore enforces on: every
/// operation authorizes against the secret's full concrete type via the impl
/// crate's `domain::authz::secret_type_resource(gts_id)`, where `gts_id` is
/// resolved from the types-registry and checked to descend from this base.
/// Registering it (and its derived types) with the types-registry is what
/// makes the resource types authorizable: the RBAC role-definitions API
/// validates `target_type` against the registry, and the authz-resolver PDP
/// compiles a tenant-scoped grant into an `InTenantSubtree` constraint on
/// `owner_tenant_id`. Without this registration every credstore operation is
/// denied (403) because no role can name the type.
///
/// Registered with an empty property set: authorization only needs the type
/// *id* known to the registry (RBAC `target_type` validation) and the PDP — the
/// tenant-scope binding comes from the impl crate's `ResourceType`
/// (`OWNER_TENANT_ID`), not from the schema body. A unit struct also sidesteps
/// the schemars-version split that breaks `JsonSchema` derivation on
/// `gts::GtsInstanceId` in this crate's dependency graph.
#[derive(Default)]
#[gts_type_schema(
    dir_path = "schemas",
    type_id = "gts.cf.core.credstore.secret.v1~",
    description = "CredStore secret resource — tenant-scoped, RBAC/PEP protected",
    properties = "",
    traits_schema = inline(SecretTypeTraits),
    // `x-gts-traits` defaults for the base type itself: generic semantics
    // (all sharing modes, opaque value). gts's `validate_entity_traits`
    // requires the base of a traits-schema chain to carry values.
    traits = serde_json::json!({
        "allow_sharing": ["private", "tenant", "shared"],
        "expirable": false,
        "utf8_only": false
    }),
    base = true
)]
pub struct SecretV1;

/// Trait schema (`x-gts-traits-schema`) carried by the secret base type,
/// and the runtime carrier of a secret type's enforceable traits.
///
/// Derived secret-type schemas (below and any registered later) declare
/// their `x-gts-traits` values against this shape; the registry validates
/// them at registration (`deny_unknown_fields` ⇒ `additionalProperties:
/// false`, so no stray trait keys). At operation time the gateway resolves
/// a type's effective traits (chain-merged, leaf wins, base fills the rest)
/// from the types-registry and deserializes them into this struct — the
/// registry is the source of truth for enforcement;
/// [`crate::types::SECRET_TYPE_CATALOG`] only seeds the built-in schemas.
#[derive(
    Debug, Clone, PartialEq, JsonSchema, serde::Serialize, serde::Deserialize, GtsTraitsSchema,
)]
#[serde(deny_unknown_fields)]
pub struct SecretTypeTraits {
    /// Sharing modes permitted for secrets of this type.
    #[serde(default)]
    #[schemars(schema_with = "sharing_modes_schema")]
    pub allow_sharing: Vec<crate::models::SharingMode>,
    /// Whether secrets of this type may carry an expiry.
    #[serde(default)]
    pub expirable: bool,
    /// Upper bound on the raw value size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size_bytes: Option<u64>,
    /// Advisory rotation cadence in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_period_secs: Option<u64>,
    /// Whether the value must be valid UTF-8.
    #[serde(default)]
    pub utf8_only: bool,
    /// JSON Schema the (UTF-8, JSON) secret value must satisfy, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_schema: Option<serde_json::Value>,
}

impl SecretTypeTraits {
    /// `true` when `mode` is permitted for this type.
    #[must_use]
    pub fn allows_sharing(&self, mode: crate::models::SharingMode) -> bool {
        self.allow_sharing.contains(&mode)
    }
}

/// Inline `{"type": "array", "items": {"type": "string", "enum": [...]}}`
/// schema for `allow_sharing`. Spelled out (instead of schemars' derived
/// `$ref` to a `oneOf`-of-`const` definition) because the GTS trait
/// validator resolves neither `$defs` references nor `oneOf` branch
/// shapes; `sharing_modes_schema_matches_enum` pins the literals to the
/// serde labels of [`crate::models::SharingMode`].
fn sharing_modes_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "items": {"type": "string", "enum": ["private", "tenant", "shared"]},
        "default": []
    })
}

// ── Secret types (derived from `SecretV1`) ──────────────────────────────────
//
// One registered GTS type schema per catalog entry in
// [`crate::types::SECRET_TYPE_CATALOG`]. Registration makes each secret type
// discoverable in the types-registry and addressable as a PDP resource type
// (per-type RBAC) without new authorization machinery, and the schema's
// `x-gts-traits` mirror the catalog traits for GTS tooling.
//
// The derived schemas are submitted as raw `InventoryTypeSchema` entries
// (not via `#[gts_type_schema(base = SecretV1, ...)]`): the macro's `base =`
// path requires a payload-generic base struct (like `PluginV1<P>`), which the
// unit `SecretV1` resource marker deliberately is not.

/// Derived-type schema JSON for a catalog entry, in the same draft-07
/// `allOf`/`$ref` shape the `gts_type_schema` macro generates, plus
/// `x-gts-traits` mirroring the catalog descriptor.
fn secret_type_schema_json(gts_id: &str) -> String {
    // Registration below only passes catalog ids; a non-catalog id is a bug
    // caught by `secret_type_gts_tests` at test time (and by this assert in
    // debug builds), so release builds fall back to the generic descriptor
    // rather than panicking in a lazy schema accessor.
    debug_assert!(
        crate::types::SecretType::from_gts_id(gts_id).is_some(),
        "secret_type_schema_json called with a non-catalog gts id: {gts_id}"
    );
    let d = crate::types::SecretType::from_gts_id(gts_id)
        .unwrap_or_else(crate::types::SecretType::generic)
        .descriptor();
    let mut traits = serde_json::json!({
        // `SharingMode` serializes to the snake_case labels the traits
        // schema's enum expects ("private" / "tenant" / "shared").
        "allow_sharing": d.allow_sharing,
        "expirable": d.expirable,
        "utf8_only": d.utf8_only,
    });
    if let Some(n) = d.max_size_bytes {
        traits["max_size_bytes"] = serde_json::json!(n);
    }
    if let Some(n) = d.rotation_period_secs {
        traits["rotation_period_secs"] = serde_json::json!(n);
    }
    if let Some(schema_src) = d.value_schema {
        // Catalog value schemas are compile-time constants pinned valid by
        // `embedded_value_schemas_are_valid_json`; on the impossible parse
        // failure the seed simply omits the trait rather than panicking.
        match serde_json::from_str::<serde_json::Value>(schema_src) {
            Ok(v) => {
                traits["value_schema"] = v;
            }
            Err(_) => debug_assert!(false, "catalog value_schema for {gts_id} is not JSON"),
        }
    }
    serde_json::json!({
        "$id": format!("gts://{gts_id}"),
        "$schema": "http://json-schema.org/draft-07/schema#",
        "allOf": [{"$ref": format!("gts://{}", SECRET_RESOURCE_TYPE)}],
        "description": format!("CredStore secret type: {}", d.name),
        "type": "object",
        "x-gts-traits": traits,
    })
    .to_string()
}

macro_rules! submit_secret_type_schema {
    ($fn_name:ident, $gts_id:expr) => {
        fn $fn_name() -> String {
            secret_type_schema_json($gts_id)
        }
        toolkit_gts::inventory::submit! {
            toolkit_gts::InventoryTypeSchema {
                type_id: $gts_id,
                schema_fn: $fn_name,
            }
        }
    };
}

submit_secret_type_schema!(
    generic_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.generic.v1~"
);
submit_secret_type_schema!(
    api_key_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.api_key.v1~"
);
submit_secret_type_schema!(
    personal_token_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.personal_token.v1~"
);
submit_secret_type_schema!(
    oauth2_client_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.oauth2_client.v1~"
);
submit_secret_type_schema!(
    basic_auth_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.basic_auth.v1~"
);
submit_secret_type_schema!(
    bearer_token_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.bearer_token.v1~"
);
submit_secret_type_schema!(
    certificate_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.certificate.v1~"
);
submit_secret_type_schema!(
    ssh_key_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.ssh_key.v1~"
);
submit_secret_type_schema!(
    webhook_hmac_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.webhook_hmac.v1~"
);
submit_secret_type_schema!(
    connection_string_schema,
    "gts.cf.core.credstore.secret.v1~cf.core.credstore.connection_string.v1~"
);

#[cfg(test)]
#[path = "gts_tests.rs"]
mod secret_type_gts_tests;
