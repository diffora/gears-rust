//! The runtime publishes the SDK catalog and configured instance before init.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use policy_engine::PolicyEngine;
use policy_engine::domain::engine_plugin;
use policy_engine_sdk::gts::permissions::Capability;
use policy_engine_sdk::{
    ASSIGNMENT_RESOURCE, BUNDLE_RESOURCE, BUNDLE_VERSION_RESOURCE, PolicyManagementClientV1,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use toolkit::Gear;
use toolkit::gts_publication::{GtsPublicationCapability as _, PreparedGtsPublication};
use types_registry_sdk::testing_platform::Call;

fn config() -> Value {
    json!({ "engine_plugin": { "vendor": "constructorfabric", "priority": 100 } })
}

#[tokio::test]
async fn the_owned_resource_types_permissions_and_configured_instance_are_published_before_init() {
    let registry = common::registry();
    let hub = Arc::new(toolkit::ClientHub::new());
    registry.install(&hub);
    let config = config();
    let provider = common::MockConfigProvider::new(&config);
    let manifest = PolicyEngine::default()
        .descriptor()
        .build_gts_manifest(&provider)
        .unwrap();
    assert_eq!(manifest.documents().len(), 7);
    PreparedGtsPublication::prepare(PolicyEngine::default().descriptor(), &provider, &hub)
        .unwrap()
        .publish(&CancellationToken::new(), Arc::new(|_: &str| {}))
        .await
        .unwrap();
    for id in [
        BUNDLE_RESOURCE,
        BUNDLE_VERSION_RESOURCE,
        ASSIGNMENT_RESOURCE,
    ] {
        assert!(registry.content(id).is_some(), "missing resource {id}");
    }
    for capability in Capability::ALL {
        let content = registry.content(capability.permission_id()).unwrap();
        assert_eq!(content["resource_type"], BUNDLE_RESOURCE);
        assert_eq!(content["action"], capability.action());
    }
    let (instance, expected) = engine_plugin::registration("constructorfabric", 100).unwrap();
    assert_eq!(registry.content(instance.as_ref()).unwrap(), expected);
    let writes = registry.calls(Call::Register);
    let ctx = common::gear_ctx_over(&config, Arc::clone(&hub)).await;
    PolicyEngine::default().init(&ctx).await.unwrap();
    assert_eq!(
        registry.calls(Call::Register),
        writes,
        "init binds without republishing"
    );
    assert!(
        hub.try_get_local::<dyn PolicyManagementClientV1>()
            .is_some()
    );
}
