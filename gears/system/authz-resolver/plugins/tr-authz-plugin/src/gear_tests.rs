use std::sync::Arc;

use authz_resolver_sdk::{AuthZResolverPluginClient, AuthZResolverPluginSpecV1};
use serde_json::json;
use tenant_resolver_sdk::TenantResolverClient;
use tokio_util::sync::CancellationToken;
use toolkit::Gear;
use toolkit::client_hub::{ClientHub, ClientScope};
use toolkit::config::ConfigProvider;
use toolkit::context::GearCtx;
use toolkit::gts::PluginV1;
use toolkit::gts_publication::{GtsManifest, GtsPublicationCapability as _};
use uuid::Uuid;

use super::TrAuthZPlugin;
use crate::domain::test_support::MockTr;

const SEGMENT: &str = "cf.builtin.tr_authz_resolver.plugin.v1";

struct Config(serde_json::Value);

impl ConfigProvider for Config {
    fn get_gear_config(&self, gear: &str) -> Option<&serde_json::Value> {
        self.0.get(gear)
    }
}

fn ctx(hub: &Arc<ClientHub>, priority: i16, cancel: CancellationToken) -> GearCtx {
    GearCtx::new(
        TrAuthZPlugin::MODULE_NAME,
        Uuid::new_v4(),
        Arc::new(Config(json!({
            TrAuthZPlugin::MODULE_NAME: { "config": {
                "vendor": "acme",
                "priority": priority,
            } }
        }))),
        Arc::clone(hub),
        cancel,
    )
}

fn instance(priority: i16) -> (String, serde_json::Value) {
    let (id, content) =
        PluginV1::<AuthZResolverPluginSpecV1>::build_registration(SEGMENT, "acme", priority)
            .expect("plugin instance serializes");
    (id.as_ref().to_owned(), content)
}

fn host() -> Arc<ClientHub> {
    let hub = Arc::new(ClientHub::new());
    hub.register::<dyn TenantResolverClient>(Arc::new(MockTr::empty()));
    hub
}

fn backend_published(hub: &ClientHub, id: &str) -> bool {
    hub.try_get_scoped::<dyn AuthZResolverPluginClient>(&ClientScope::gts_id(id))
        .is_some()
}

/// The manifest the runtime publishes for `priority`, without `init`.
fn manifest(priority: i16) -> GtsManifest {
    TrAuthZPlugin::default()
        .descriptor()
        .build_gts_manifest(&Config(json!({
            TrAuthZPlugin::MODULE_NAME: { "config": { "vendor": "acme", "priority": priority } }
        })))
        .expect("manifest")
}

/// `init` with the admitted manifest for `priority`, as the runtime calls it.
async fn admitted_init(hub: &Arc<ClientHub>, priority: i16) -> anyhow::Result<()> {
    TrAuthZPlugin::default()
        .init(
            &ctx(hub, priority, CancellationToken::new())
                .with_gts_publication_manifest_for_test(Arc::new(manifest(priority))),
        )
        .await
}

#[test]
fn the_manifest_is_the_configured_instance() {
    assert_eq!(
        manifest(7).documents(),
        [instance(7)]
            .map(|(id, document)| (types_registry_sdk::GtsId::try_new(&id).unwrap(), document))
    );
}

#[tokio::test]
async fn init_binds_the_admitted_instance() {
    let hub = host();

    admitted_init(&hub, 7)
        .await
        .expect("init with an admitted manifest");

    assert!(backend_published(&hub, &instance(7).0));
}

/// Without the tenant resolver the plugin could not evaluate anything: its
/// `init` fails before binding.
#[tokio::test]
async fn a_host_without_the_tenant_resolver_fails_init_before_binding() {
    let hub = Arc::new(ClientHub::new());
    let (id, _) = instance(7);

    admitted_init(&hub, 7)
        .await
        .expect_err("no tenant resolver");

    assert!(!backend_published(&hub, &id));
}

/// Outside the runtime there is no admitted manifest: nothing is bound.
#[tokio::test]
async fn init_without_an_admitted_manifest_binds_nothing() {
    let hub = host();
    let (id, _) = instance(7);

    let error = TrAuthZPlugin::default()
        .init(&ctx(&hub, 7, CancellationToken::new()))
        .await
        .expect_err("no admitted manifest");

    assert!(
        format!("{error:#}").contains("admitted GTS manifest"),
        "{error:#}"
    );
    assert!(!backend_published(&hub, &id));
}
