use serde_json::json;
use toolkit_gts::{gts_id, gts_uri};
use types_registry_sdk::GtsTypeId;

use super::*;

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
