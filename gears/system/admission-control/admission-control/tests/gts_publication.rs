//! GTS declarations are published and admitted before the application can init.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use admission_control::AdmissionControl;
use admission_control_sdk::{ADMISSION_CONTROL_RESOURCE, AUDIT_TOPIC_ID, REFUSAL_EVENT_TYPE};
use event_broker_sdk::gts::{EventV1, TopicV1};
use gts::GtsSchema;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use toolkit::Gear;
use toolkit::gts_publication::{GtsPublicationCapability as _, PreparedGtsPublication};
use toolkit_gts::PluginV1;
use types_registry_sdk::GtsId;
use types_registry_sdk::testing_platform::{Call, MockTypesRegistry};

fn provider() -> common::MockConfigProvider {
    common::MockConfigProvider::new(&json!({}))
}

#[test]
fn manifest_contains_exactly_the_four_owned_sdk_declarations() {
    let manifest = AdmissionControl::default()
        .descriptor()
        .build_gts_manifest(&provider())
        .unwrap();
    let expected: Vec<(GtsId, serde_json::Value)> = admission_control_sdk::gts_declarations()
        .into_iter()
        .map(|(id, text)| {
            (
                GtsId::try_new(&id).unwrap(),
                serde_json::from_str(&text).unwrap(),
            )
        })
        .collect();
    assert_eq!(manifest.documents(), expected);
    assert_eq!(manifest.documents().len(), 4);
    for id in [
        ADMISSION_CONTROL_RESOURCE,
        AUDIT_TOPIC_ID,
        REFUSAL_EVENT_TYPE,
    ] {
        assert!(
            manifest
                .documents()
                .iter()
                .any(|(declared, _)| declared.id() == id),
            "missing {id}"
        );
    }
}

#[tokio::test]
async fn init_after_publication_registers_the_client_without_publishing_again() {
    let registry = Arc::new(MockTypesRegistry::new());
    // Supply platform prerequisites, leaving the gear's own declarations to publication.
    for (id, schema) in [
        (
            PluginV1::<()>::TYPE_ID,
            PluginV1::<()>::gts_schema_with_refs(),
        ),
        (
            EventV1::<()>::TYPE_ID,
            EventV1::<()>::gts_schema_with_refs(),
        ),
        (TopicV1::TYPE_ID, TopicV1::gts_schema_with_refs()),
    ] {
        registry.seed(id, schema);
    }
    let hub = Arc::new(toolkit::ClientHub::new());
    registry.install(&hub);
    let manifest = Arc::new(
        AdmissionControl::default()
            .descriptor()
            .build_gts_manifest(&provider())
            .unwrap(),
    );
    let publication = PreparedGtsPublication::bind(
        AdmissionControl::default().descriptor(),
        Arc::clone(&manifest),
        toolkit::gts_publication::GtsPublicationOptions::default(),
        &hub,
    )
    .unwrap();
    publication
        .publish(&CancellationToken::new(), Arc::new(|_: &str| {}))
        .await
        .unwrap();
    let registered = registry.calls(Call::Register);
    assert!(registered > 0);
    let ctx = toolkit::GearCtx::new(
        common::GEAR,
        uuid::Uuid::new_v4(),
        Arc::new(provider()),
        Arc::clone(&hub),
        CancellationToken::new(),
    )
    .with_gts_publication_manifest_for_test(manifest);
    AdmissionControl::default().init(&ctx).await.unwrap();
    assert!(
        hub.try_get::<dyn admission_control_sdk::AdmissionClientV1>()
            .is_some()
    );
    assert_eq!(registry.calls(Call::Register), registered);
    assert_eq!(
        registry.content(REFUSAL_EVENT_TYPE).unwrap(),
        admission_control_sdk::gts::refusal_event_type_schema()
    );
}
