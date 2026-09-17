//! Mapping probes for [`super::catalog_sku_of`]: the field pairing pricing's
//! registry contract reads, and the three refusals a rule would otherwise
//! key on a fabricated empty / default.
#![allow(clippy::expect_used)]

use bss_pricing_sdk::product_catalog::CatalogSku;
use uuid::Uuid;

use super::{MappingError, catalog_sku_of};
use crate::infra::storage::entity::read_entity;

const SKU_ID: Uuid = Uuid::from_u128(0xca_7a_10_91);
const TENANT: Uuid = Uuid::from_u128(0xca_7a_10_01);

/// A published SKU serving row. `name` is the code because the projector
/// fills `name` from `sku_code` — pin that, do not invent a display label.
fn published_sku_row() -> read_entity::Model {
    read_entity::Model {
        tenant_id: TENANT,
        entity_kind: "sku".to_owned(),
        entity_id: SKU_ID,
        entity_code: Some("COMP-VCPU-H".to_owned()),
        name: "COMP-VCPU-H".to_owned(),
        lifecycle_state: "published".to_owned(),
        deprecated: false,
        composition_pending: false,
        sellable: Some(true),
        deprecation_provenance: None,
        replaced_by_sku_id: None,
        region_scope: String::new(),
        brand_scope: String::new(),
        sku_type: Some("service".to_owned()),
        plan_tier_label: Some("Pro".to_owned()),
        metering_unit: Some("vCPU-hour".to_owned()),
        usage_type_ref: Some("cf.usage.vcpu-hour".to_owned()),
        display_attributes: None,
        category_paths: None,
        published_version: 3,
        projected_at: crate::test_support::utc(2026, 9, 17, 12, 0, 0),
        generation: 1,
    }
}

#[test]
fn a_published_sku_row_maps_field_for_field() {
    let row = published_sku_row();
    let mapped = catalog_sku_of(&row).expect("a complete published sku maps");
    assert_eq!(
        mapped,
        CatalogSku {
            sku_id: SKU_ID,
            sku_code: "COMP-VCPU-H".to_owned(),
            name: "COMP-VCPU-H".to_owned(),
            metering_unit: Some("vCPU-hour".to_owned()),
            status: "published".to_owned(),
            plan_tier: Some("Pro".to_owned()),
            sku_type: "service".to_owned(),
            sellable: true,
            usage_type_ref: Some("cf.usage.vcpu-hour".to_owned()),
            deprecated: false,
        }
    );
    // The projector fills `name` from `sku_code`. A display label here would
    // be invented: this IS the code.
    assert_eq!(mapped.name, mapped.sku_code);
    assert_eq!(mapped.name, row.name);
}

#[test]
fn a_product_row_is_skipped_not_mapped() {
    let mut row = published_sku_row();
    row.entity_kind = "product".to_owned();
    row.entity_code = Some("PROD-1".to_owned());
    row.name = "A product display name".to_owned();
    row.sku_type = None;
    row.sellable = None;
    assert_eq!(catalog_sku_of(&row), Err(MappingError::NotASku));
}

#[test]
fn a_missing_entity_code_is_a_mapping_error_not_an_empty_code() {
    let mut row = published_sku_row();
    row.entity_code = None;
    assert_eq!(catalog_sku_of(&row), Err(MappingError::MissingSkuCode));
    assert!(
        !matches!(catalog_sku_of(&row), Ok(CatalogSku { sku_code, .. }) if sku_code.is_empty()),
        "None must not become an empty sku_code"
    );
}

#[test]
fn a_missing_sellable_is_a_mapping_error() {
    let mut row = published_sku_row();
    row.sellable = None;
    assert_eq!(catalog_sku_of(&row), Err(MappingError::MissingSellable));
}

#[test]
fn a_missing_sku_type_is_a_mapping_error() {
    let mut row = published_sku_row();
    row.sku_type = None;
    assert_eq!(catalog_sku_of(&row), Err(MappingError::MissingSkuType));
}
