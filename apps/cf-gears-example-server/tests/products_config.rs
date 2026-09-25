//! Load the deployment YAML through the server path, then resolve products defaults.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![cfg(feature = "bss-products")]

#[test]
fn e2e_yaml_needs_no_products_config_section() {
    use std::sync::Arc;
    use toolkit::bootstrap::AppConfig;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/e2e-local.yaml");
    let config = AppConfig::load_or_default(Some(&path)).unwrap();
    assert!(!config.gears.contains_key("bss-products"));
    let ctx = toolkit::GearCtx::new(
        "bss-products",
        uuid::Uuid::nil(),
        Arc::new(config),
        Arc::new(toolkit::ClientHub::new()),
        tokio_util::sync::CancellationToken::new(),
    );
    // This is the same typed loader and validation that products init calls.
    let products: bss_products::config::ProductsConfig = ctx.config_or_default().unwrap();
    products.validate().unwrap();
    assert!(products.reference_principals.is_empty());
    assert!(products.resolved_idempotency_retention_hours() > 0);
}
