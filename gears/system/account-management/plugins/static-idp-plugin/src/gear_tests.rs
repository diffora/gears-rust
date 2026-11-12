//! The publication boundary: the runtime publishes the configured instance
//! before `init`; `init` binds the scoped client to the admitted manifest.

use std::sync::Arc;

use account_management_sdk::{IdpPluginClient, IdpPluginSpecV1};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use toolkit::Gear;
use toolkit::client_hub::{ClientHub, ClientScope};
use toolkit::config::ConfigProvider;
use toolkit::context::GearCtx;
use toolkit::gts::PluginV1;
use toolkit::gts_publication::GtsPublicationCapability;
use uuid::Uuid;

use super::StaticIdpPlugin;

const SEGMENT: &str = "cf.builtin.static_idp.plugin.v1";

struct Config(serde_json::Value);

impl ConfigProvider for Config {
    fn get_gear_config(&self, gear: &str) -> Option<&serde_json::Value> {
        self.0.get(gear)
    }
}

fn config(priority: i16) -> Config {
    Config(json!({
        StaticIdpPlugin::MODULE_NAME: { "config": { "vendor": "cf", "priority": priority } }
    }))
}

fn ctx(hub: &Arc<ClientHub>, priority: i16) -> GearCtx {
    GearCtx::new(
        StaticIdpPlugin::MODULE_NAME,
        Uuid::new_v4(),
        Arc::new(config(priority)),
        Arc::clone(hub),
        CancellationToken::new(),
    )
}

fn instance(priority: i16) -> (String, serde_json::Value) {
    let (id, content) = PluginV1::<IdpPluginSpecV1>::build_registration(SEGMENT, "cf", priority)
        .expect("plugin instance serializes");
    (id.as_ref().to_owned(), content)
}

fn bound(hub: &ClientHub, id: &str) -> bool {
    hub.try_get_scoped::<dyn IdpPluginClient>(&ClientScope::gts_id(id))
        .is_some()
}

/// Without an admitted manifest — outside the runtime — `init` fails and
/// binds nothing.
#[tokio::test]
async fn init_without_an_admitted_manifest_binds_nothing() {
    let hub = Arc::new(ClientHub::new());

    let err = StaticIdpPlugin::default()
        .init(&ctx(&hub, 7))
        .await
        .unwrap_err();

    assert!(
        format!("{err:#}").contains("admitted GTS manifest"),
        "{err:#}"
    );
    assert!(!bound(&hub, &instance(7).0));
}

/// The manifest is the configured instance, without `init`.
#[test]
fn the_manifest_is_the_configured_instance() {
    let gear = StaticIdpPlugin::default();
    let manifest = gear.descriptor().build_gts_manifest(&config(7)).unwrap();

    assert_eq!(
        manifest.documents(),
        [instance(7)]
            .map(|(id, document)| (types_registry_sdk::GtsId::try_new(&id).unwrap(), document))
    );
    assert_eq!(manifest.publisher_name(), "static-idp-plugin");
    assert_eq!(manifest.publisher_version(), env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn init_binds_the_admitted_instance() {
    let hub = Arc::new(ClientHub::new());
    let gear = StaticIdpPlugin::default();
    let manifest = Arc::new(gear.descriptor().build_gts_manifest(&config(7)).unwrap());

    gear.init(&ctx(&hub, 7).with_gts_publication_manifest_for_test(manifest))
        .await
        .expect("init with an admitted manifest");

    assert!(bound(&hub, &instance(7).0));
}
