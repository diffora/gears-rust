//! The GTS publication `gts_publication(...)` declares: exactly the SDK's declarations, under
//! this gear's name and version, independent of configuration.

#![allow(clippy::expect_used)]

use toolkit::config::ConfigProvider;
use toolkit::gts_publication::GtsPublicationCapability as _;

use super::*;

struct NoConfig;

impl ConfigProvider for NoConfig {
    fn get_gear_config(&self, _gear: &str) -> Option<&serde_json::Value> {
        None
    }
}

#[test]
fn the_manifest_is_the_sdk_declarations() {
    let manifest = AuthZResolver
        .descriptor()
        .build_gts_manifest(&NoConfig)
        .expect("manifest");

    assert_eq!(manifest.publisher_name(), "authz-resolver");
    assert_eq!(manifest.publisher_version(), env!("CARGO_PKG_VERSION"));
    let ids: Vec<&str> = manifest.documents().iter().map(|(id, _)| id.id()).collect();
    let declared: Vec<String> = authz_resolver_sdk::gts_declarations()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids, declared);
    assert!(
        ids.contains(&authz_resolver_sdk::AuthZResolverPluginSpecV1::gts_type_id().as_ref()),
        "{ids:?}"
    );
}
