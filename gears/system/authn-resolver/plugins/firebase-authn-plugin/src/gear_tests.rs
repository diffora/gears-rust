//! The plugin must publish into the platform catalogue the migrated resolver reads.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use authn_resolver_sdk::{AuthNResolverPluginClient, AuthNResolverPluginSpecV1};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use toolkit::client_hub::{ClientHub, ClientScope};
use toolkit::gts::PluginV1;
use toolkit::gts_publication::{
    GtsManifest, GtsPublicationCapability as _, PreparedGtsPublication,
};
use toolkit::{ConfigProvider, Gear, GearCtx};
use toolkit_security::PlatformSecurityContext;
use types_registry_sdk::testing_platform::MockTypesRegistry;
use types_registry_sdk::{GtsIdPattern, ListEntitiesRequest, PlatformTypesRegistryApiExt};

use super::{FirebaseAuthnPlugin, PLUGIN_SUFFIX};

struct Config(Value);

impl ConfigProvider for Config {
    fn get_gear_config(&self, _gear: &str) -> Option<&Value> {
        Some(&self.0)
    }
}

fn config(vendor: &str, priority: i16) -> Config {
    Config(json!({ "config": {
        "vendor": vendor,
        "priority": priority,
        "project_id": "firebase-authn-test",
        "identities": [{
            "uid": "account-a", "email": "a@example.test",
            "tenant_id": "00000000-0000-0000-0000-000000000001",
            "token_scopes": ["scope:a"]
        }]
    }}))
}

fn instance_id() -> String {
    PluginV1::<AuthNResolverPluginSpecV1>::build_registration(PLUGIN_SUFFIX, "", 0)
        .unwrap()
        .0
        .to_string()
}

fn bound(hub: &ClientHub) -> bool {
    hub.try_get_scoped::<dyn AuthNResolverPluginClient>(&ClientScope::gts_id(&instance_id()))
        .is_some()
}

fn hub(registry: &Arc<MockTypesRegistry>) -> Arc<ClientHub> {
    let hub = Arc::new(ClientHub::new());
    registry.install(&hub);
    hub
}

fn ctx(hub: Arc<ClientHub>, config: Config, manifest: Option<Arc<GtsManifest>>) -> GearCtx {
    let ctx = GearCtx::new(
        "firebase-authn-plugin",
        uuid::Uuid::new_v4(),
        Arc::new(config),
        hub,
        CancellationToken::new(),
    );
    match manifest {
        Some(manifest) => ctx.with_gts_publication_manifest_for_test(manifest),
        None => ctx,
    }
}

/// Publish through the Firebase descriptor and return the manifest that was admitted.
async fn prepare(hub: &ClientHub, config: &Config) -> anyhow::Result<Arc<GtsManifest>> {
    let publication =
        PreparedGtsPublication::prepare(FirebaseAuthnPlugin::default().descriptor(), config, hub)?;
    publication
        .publish(&CancellationToken::new(), Arc::new(|_: &str| {}))
        .await?;
    Ok(publication.shared_gts_manifest())
}

#[tokio::test]
async fn the_instance_is_discoverable_in_the_platform_catalogue_after_publication() {
    let registry = Arc::new(MockTypesRegistry::new());
    let hub = hub(&registry);
    let config = config("cf", 7);
    let manifest = prepare(&hub, &config).await.unwrap();
    FirebaseAuthnPlugin::default()
        .init(&ctx(Arc::clone(&hub), config, Some(manifest)))
        .await
        .unwrap();

    let listed = registry
        .list_instances(
            &PlatformSecurityContext::outbound_marker(),
            ListEntitiesRequest::matching(
                GtsIdPattern::try_new(AuthNResolverPluginSpecV1::gts_type_id().as_ref()).unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        listed.len(),
        1,
        "the migrated resolver reads this catalogue"
    );
    let content = listed[0].content.as_ref().unwrap();
    assert_eq!(content["vendor"], "cf");
    assert_eq!(content["priority"], 7);
    assert!(content.get("identities").is_none());
    assert!(content.get("project_id").is_none());
    let selected = toolkit::plugins::choose_registered_plugin_instance::<
        AuthNResolverPluginSpecV1,
        dyn AuthNResolverPluginClient,
    >(
        &hub,
        "cf",
        listed.iter().map(|i| (i.id.as_ref(), i.content.as_ref())),
    )
    .unwrap();
    assert_eq!(selected, instance_id());
}

#[tokio::test]
async fn init_without_an_admitted_manifest_binds_nothing() {
    let hub = Arc::new(ClientHub::new());
    let result = FirebaseAuthnPlugin::default()
        .init(&ctx(Arc::clone(&hub), config("cf", 7), None))
        .await;
    assert!(result.is_err(), "init requires an admitted manifest");
    assert!(!bound(&hub));
}

/// Publication reads vendor and priority only: neither a project variable the
/// publishing process lacks nor the identity mappings stop it.
#[tokio::test]
async fn publication_reads_neither_runtime_values_nor_identities() {
    let registry = Arc::new(MockTypesRegistry::new());
    let hub = hub(&registry);
    let mut config = config("cf", 7);
    config.0["config"]["project_id"] = json!("${CF_GEARS_TEST_UNSET_FIREBASE_PROJECT}");
    config.0["config"]["identities"] = json!([]);

    let manifest = prepare(&hub, &config).await.unwrap();

    let documents = manifest.documents();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].1["vendor"], "cf");
    assert_eq!(documents[0].1["priority"], 7);
    assert!(registry.content(&instance_id()).is_some());
}

/// The full configuration is still expanded and validated by `init`, which binds
/// nothing when it is refused.
#[tokio::test]
async fn invalid_identity_configuration_fails_init_and_binds_nothing() {
    let registry = Arc::new(MockTypesRegistry::new());
    let hub = hub(&registry);
    let mut config = config("cf", 7);
    config.0["config"]["identities"] = json!([]);
    let manifest = prepare(&hub, &config).await.unwrap();

    let error = FirebaseAuthnPlugin::default()
        .init(&ctx(Arc::clone(&hub), config, Some(manifest)))
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("identity mappings are required"));
    assert!(!bound(&hub));
}

#[tokio::test]
async fn unexpanded_project_fails_init_and_binds_nothing() {
    let registry = Arc::new(MockTypesRegistry::new());
    let hub = hub(&registry);
    let mut config = config("cf", 7);
    config.0["config"]["project_id"] = json!("${CF_GEARS_TEST_UNSET_FIREBASE_PROJECT}");
    let manifest = prepare(&hub, &config).await.unwrap();

    let error = FirebaseAuthnPlugin::default()
        .init(&ctx(Arc::clone(&hub), config, Some(manifest)))
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("CF_GEARS_TEST_UNSET_FIREBASE_PROJECT"));
    assert!(!bound(&hub));
}
