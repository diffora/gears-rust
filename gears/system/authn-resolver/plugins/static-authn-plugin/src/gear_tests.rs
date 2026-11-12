//! The publication boundary: the runtime publishes the config-only instance
//! before `init`, and `init` binds the scoped client to the admitted
//! manifest. Nothing binds without an admitted manifest.

use std::sync::Arc;

use authn_resolver_sdk::{AuthNResolverPluginClient, AuthNResolverPluginSpecV1};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use toolkit::client_hub::{ClientHub, ClientScope};
use toolkit::contracts::Gear;
use toolkit::gts::PluginV1;
use toolkit::gts_publication::{GtsManifest, GtsPublicationCapability as _};
use toolkit::{ConfigProvider, GearCtx};

use super::StaticAuthNPlugin;

const INSTANCE: &str = "cf.builtin.static_authn_resolver.plugin.v1";

struct FixedConfig(Value);

impl ConfigProvider for FixedConfig {
    fn get_gear_config(&self, _gear: &str) -> Option<&Value> {
        Some(&self.0)
    }
}

fn config_of(config: &Value) -> FixedConfig {
    FixedConfig(json!({ "config": config }))
}

fn instance_id() -> String {
    PluginV1::<AuthNResolverPluginSpecV1>::build_registration(INSTANCE, "", 0)
        .unwrap()
        .0
        .as_ref()
        .to_owned()
}

fn bound(hub: &ClientHub) -> bool {
    hub.try_get_scoped::<dyn AuthNResolverPluginClient>(&ClientScope::gts_id(&instance_id()))
        .is_some()
}

/// The manifest the runtime publishes for `config`, before any `init`.
fn manifest(config: &Value) -> GtsManifest {
    StaticAuthNPlugin::default()
        .descriptor()
        .build_gts_manifest(&config_of(config))
        .expect("manifest")
}

/// A context for `init`, carrying `admitted` as the runtime would.
fn ctx(config: &Value, admitted: Option<GtsManifest>) -> GearCtx {
    let ctx = GearCtx::new(
        "static-authn-plugin",
        uuid::Uuid::new_v4(),
        Arc::new(config_of(config)),
        Arc::new(ClientHub::new()),
        CancellationToken::new(),
    );
    match admitted {
        Some(manifest) => ctx.with_gts_publication_manifest_for_test(Arc::new(manifest)),
        None => ctx,
    }
}

/// The manifest is config-only: the configured instance, no `init` needed.
#[test]
fn the_manifest_is_the_configured_instance() {
    let manifest = manifest(&json!({ "vendor": "cf", "priority": 7 }));

    let documents = manifest.documents();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].0.id(), instance_id());
    assert_eq!(documents[0].1["vendor"], "cf");
    assert_eq!(documents[0].1["priority"], 7);
}

/// A configuration whose S2S secret names a variable only the serving process has.
fn with_runtime_secret() -> Value {
    json!({
        "vendor": "cf",
        "priority": 7,
        "s2s_credentials": [{
            "client_id": "svc",
            "client_secret": "${CF_GEARS_TEST_UNSET_STATIC_AUTHN_SECRET}"
        }]
    })
}

/// The manifest reads vendor and priority only: a publishing process without the
/// server's secret environment still builds it.
#[test]
fn the_manifest_does_not_expand_runtime_secrets() {
    let manifest = manifest(&with_runtime_secret());

    let documents = manifest.documents();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].1["vendor"], "cf");
    assert_eq!(documents[0].1["priority"], 7);
}

/// The secret is still expanded, and its absence refused, by `init`.
#[tokio::test]
async fn init_refuses_an_unexpanded_runtime_secret() {
    let config = with_runtime_secret();
    let ctx = ctx(&config, Some(manifest(&config)));

    let err = StaticAuthNPlugin::default().init(&ctx).await.unwrap_err();

    assert!(
        format!("{err:#}").contains("CF_GEARS_TEST_UNSET_STATIC_AUTHN_SECRET"),
        "{err:#}"
    );
    assert!(!bound(&ctx.client_hub()));
}

#[tokio::test]
async fn init_binds_the_admitted_instance() {
    let config = json!({ "vendor": "cf", "priority": 7 });
    let ctx = ctx(&config, Some(manifest(&config)));

    StaticAuthNPlugin::default().init(&ctx).await.expect("init");

    assert!(bound(&ctx.client_hub()));
}

/// Without an admitted manifest — outside the runtime — `init` fails and
/// binds nothing; it never publishes.
#[tokio::test]
async fn init_without_an_admitted_manifest_binds_nothing() {
    let ctx = ctx(&json!({}), None);

    let err = StaticAuthNPlugin::default().init(&ctx).await.unwrap_err();

    assert!(
        format!("{err:#}").contains("admitted GTS manifest"),
        "{err:#}"
    );
    assert!(!bound(&ctx.client_hub()));
}

/// A repeated `init` is refused.
#[tokio::test]
async fn a_second_init_is_refused() {
    let config = json!({ "vendor": "cf" });
    let gear = StaticAuthNPlugin::default();
    gear.init(&ctx(&config, Some(manifest(&config))))
        .await
        .expect("first init");

    let err = gear
        .init(&ctx(&config, Some(manifest(&config))))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("already initialized"), "{err}");
}
