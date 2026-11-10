//! AM's configured platform-root tenant type, published by the `startup` task with
//! AM's declarations, before bootstrap. Equal content submits nothing; permitted
//! `idp_provisioning` changes use normal compatibility checks.
//!
//! The caller must validate the stored root binding before any registry write.

use serde_json::Value;
use toolkit_gts::GTS_ID_URI_PREFIX;

use crate::domain::root_type::RootTypeConfig;
use crate::gts::root_type::PlatformRootTypeV1;

/// Build the single AM-owned concrete root schema.
pub fn desired_root_schema(cfg: &RootTypeConfig) -> anyhow::Result<Value> {
    // The configuration guarantees a type directly derived from the envelope.
    let type_id = cfg.validated_id().map_err(anyhow::Error::msg)?;
    let mut schema: Value =
        serde_json::from_str(&PlatformRootTypeV1::gts_schema_with_refs_as_string())?;
    // Only declaration metadata is configurable; the macro owns the schema body
    // and the root's fixed empty allowed_parent_types trait.
    schema["$id"] = format!("{GTS_ID_URI_PREFIX}{type_id}").into();
    schema["x-gts-traits"]["idp_provisioning"] = cfg.idp_provisioning.into();
    Ok(schema)
}

#[cfg(test)]
#[path = "root_type_tests.rs"]
mod tests;
