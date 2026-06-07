//! GTS (Global Type System) declarations for the credstore SDK.
//!
//! Credstore type/resource ids follow the platform convention
//! `gts.cf.core.credstore.<resource>.v1~`: the trailing `~` terminates the type
//! URI and `.v1` is the schema version. Plugin specs nest under the modkit base,
//! `gts.cf.modkit.plugins.plugin.v1~cf.core.credstore.<spec>.v1~`.
//!
//! Each `#[gts_type_schema(...)]`-annotated struct submits its schema to the
//! link-time inventory the types-registry loads at boot. Registration is what
//! makes a resource type authorizable: RBAC validates a role's `target_type`
//! against the registry and the PDP compiles a tenant-scoped grant into an
//! `InTenantSubtree` constraint on `owner_tenant_id`. The same string is the
//! single source of truth for the canonical-error `resource_type` — see
//! [`SECRET_RESOURCE_TYPE`], pinned to the impl crate's `#[resource_error(...)]`
//! marker by a unit test.

use modkit::gts::PluginV1;
use modkit_gts::gts_type_schema;

/// GTS resource-type id for the credstore **secret** — the single source of
/// truth for this string. Mirrored by [`SecretV1`]'s `schema_id`, the impl
/// crate's `domain::authz::SECRET` PEP `ResourceType`, and the
/// `#[resource_error(...)]` marker in `infra::sdk_error_mapping` (a unit test
/// there pins that marker to this constant so a divergence trips at test time).
pub const SECRET_RESOURCE_TYPE: &str = "gts.cf.core.credstore.secret.v1~";

#[derive(Default)]
#[gts_type_schema(
    dir_path = "schemas",
    base = PluginV1,
    schema_id = "gts.cf.modkit.plugins.plugin.v1~cf.core.credstore.plugin.v1~",
    description = "CredStore plugin specification",
    properties = "",
)]
pub struct CredStorePluginSpecV1;

/// GTS type-schema for the credstore **secret** resource.
///
/// This is the PEP `ResourceType` credstore enforces on
/// (`gts.cf.core.credstore.secret.v1~`, see the impl crate's
/// `domain::authz::SECRET`). Registering it with the types-registry is what
/// makes the resource type authorizable: the RBAC role-definitions API
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
    schema_id = "gts.cf.core.credstore.secret.v1~",
    description = "CredStore secret resource — tenant-scoped, RBAC/PEP protected",
    properties = "",
    base = true
)]
pub struct SecretV1;
