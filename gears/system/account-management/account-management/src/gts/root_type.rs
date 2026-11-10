//! Configured-only root tenant-type template; deliberately absent from inventory.

use account_management_sdk::gts::{TenantTypeTraitsV1, TenantTypeV1};
use toolkit_gts::gts_type_schema_template;

// The template must not register its fixed ID when a deployment configures
// another ID or no root contract. The startup manifest owns publication.
/// Platform-root tenant-type schema template with no parents or `IdP` provisioning.
#[gts_type_schema_template(
    dir_path = "schemas",
    base = TenantTypeV1,
    type_id = gts_id!("cf.core.am.tenant_type.v1~cf.core.am.platform.v1~"),
    description = "Platform-root tenant type (no parents).",
    properties = "",
    traits = TenantTypeTraitsV1::builder()
        .with_allowed_parent_types(Vec::<String>::new())
        .with_idp_provisioning(false)
        .build()
)]
// This type adds behavioral traits, not payload restrictions. Preserve the
// existing envelope extension point rather than closing an empty unit payload.
#[schemars(extend("additionalProperties" = true))]
pub struct PlatformRootTypeV1;
