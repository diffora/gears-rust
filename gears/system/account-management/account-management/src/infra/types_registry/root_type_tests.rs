use serde_json::json;
use toolkit_gts::{gts_id, gts_uri};
use types_registry_sdk::GtsTypeId;

use super::*;
use crate::domain::root_type::TENANT_TYPE_BASE;

const ROOT_TYPE: &str = gts_id!("cf.core.am.tenant_type.v1~cf.core.am.platform.v1~");

fn config(idp_provisioning: bool) -> RootTypeConfig {
    RootTypeConfig {
        gts_id: GtsTypeId::new(ROOT_TYPE),
        idp_provisioning,
    }
}

#[test]
fn desired_schema_is_the_am_owned_root_contract() {
    let desired = desired_root_schema(&config(true)).expect("desired schema");

    assert_eq!(
        desired["$id"],
        json!(gts_uri!(
            "cf.core.am.tenant_type.v1~cf.core.am.platform.v1~"
        ))
    );
    assert_eq!(
        desired["$schema"],
        json!("http://json-schema.org/draft-07/schema#")
    );
    assert_eq!(
        desired["description"],
        json!("Platform-root tenant type (no parents).")
    );
    assert_eq!(desired["type"], json!("object"));
    assert_eq!(
        desired.pointer("/allOf/0/$ref"),
        Some(&json!(gts_uri!("cf.core.am.tenant_type.v1~")))
    );
    assert_eq!(
        desired.pointer("/x-gts-traits/allowed_parent_types"),
        Some(&json!([]))
    );
    assert_eq!(
        desired.pointer("/x-gts-traits/idp_provisioning"),
        Some(&json!(true))
    );
}

#[test]
fn root_schema_template_is_not_an_automatic_declaration() {
    let template_id = crate::gts::root_type::PlatformRootTypeV1::gts_type_id();
    assert!(
        crate::gts_declarations()
            .iter()
            .all(|(id, _)| id != template_id.as_ref()),
        "a configured-only root must not be published by the crate collector"
    );
    assert!(
        toolkit_gts::inventory::iter::<toolkit_gts::InventoryTypeSchema>
            .into_iter()
            .all(|entry| entry.type_id != template_id.as_ref()),
        "a configured-only root must not be seeded by the global inventory"
    );
}

#[test]
fn generated_root_preserves_the_previous_open_payload_contract() {
    use account_management_sdk::gts::TenantTypeV1;
    use gts::GtsSchema;

    let mut store = gts::GtsStore::new();
    store
        .register_schema(
            TENANT_TYPE_BASE,
            &TenantTypeV1::<()>::gts_schema_with_refs(),
        )
        .expect("base schema");
    // Previously published document: the root only inherits the open envelope.
    let previous = json!({
        "$id": gts_uri!("cf.core.am.tenant_type.v1~cf.core.am.platform.v1~"),
        "$schema": "http://json-schema.org/draft-07/schema#",
        "description": "Platform-root tenant type (no parents).",
        "type": "object",
        "allOf": [{ "$ref": gts_uri!("cf.core.am.tenant_type.v1~") }],
        "x-gts-traits": {"allowed_parent_types": [], "idp_provisioning": false}
    });
    let candidate = desired_root_schema(&config(false)).expect("candidate");
    let comparison = store
        .compare_documents(&previous, &candidate)
        .expect("resolved comparison");
    assert!(
        comparison.full_compatibility().is_compatible(),
        "{comparison:?}"
    );
    store
        .register_schema(ROOT_TYPE, &candidate)
        .expect("root schema");
    store
        .validate_schema_with(ROOT_TYPE, gts::GtsRefValidation::None)
        .expect("native admission");
    store
        .validate_payload_with(
            ROOT_TYPE,
            &json!({"payload": {"custom": {"nested": true}}}),
            gts::GtsRefValidation::None,
        )
        .expect("existing payload extension point stays open");
}
