//! Mapping probes for [`super::catalog_sku_of`], and the two reads over a
//! real projection so `get_skus` cannot serve a wider set than browse.
#![allow(clippy::expect_used)]

use bss_pricing_sdk::product_catalog::{CatalogSku, ProductCatalogClientV1};
use sea_orm::EntityTrait;
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::{AccessScope, SecureInsertExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{BrowseCatalogProvider, MappingError, catalog_sku_of};
use crate::infra::storage::entity::read_entity;
use crate::infra::storage::migrations::Migrator;

const SKU_ID: Uuid = Uuid::from_u128(0xca_7a_10_91);
const DEPRECATED_ID: Uuid = Uuid::from_u128(0xca_7a_10_92);
const DRAFT_ID: Uuid = Uuid::from_u128(0xca_7a_10_93);
const RETIRED_ID: Uuid = Uuid::from_u128(0xca_7a_10_94);
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
        sku_type: Some("offer".to_owned()),
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
            sku_type: "offer".to_owned(),
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

#[test]
fn a_deprecated_serving_row_maps_status_published_and_the_flag() {
    let mut row = published_sku_row();
    row.lifecycle_state = "deprecated".to_owned();
    row.deprecated = true;
    let mapped = catalog_sku_of(&row).expect("a deprecated serving sku maps");
    assert_eq!(mapped.status, "published");
    assert!(mapped.deprecated);
}

async fn projection_harness() -> DBProvider<DbError> {
    let db = connect_db(
        "sqlite::memory:",
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot the migration chain");
    DBProvider::<DbError>::new(db)
}

async fn insert_projection_row(db: &DBProvider<DbError>, row: read_entity::Model) {
    let conn = db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(row.tenant_id);
    let model: read_entity::ActiveModel = row.into();
    read_entity::Entity::insert(model.clone())
        .secure()
        .scope_with_model(&scope, &model)
        .expect("scope the insert")
        .exec(&conn)
        .await
        .expect("insert a projection row");
}

fn sku_row(
    entity_id: Uuid,
    code: &str,
    lifecycle_state: &str,
    deprecated: bool,
) -> read_entity::Model {
    let mut row = published_sku_row();
    row.entity_id = entity_id;
    row.entity_code = Some(code.to_owned());
    row.name = code.to_owned();
    row.lifecycle_state = lifecycle_state.to_owned();
    row.deprecated = deprecated;
    // Search walks the checkpoint's serving generation, which is 0 before
    // the first projector pass — match that so the seed is visible.
    row.generation = 0;
    row
}

/// Every role reaches Pricing intact, and the sale flag travels beside it
/// without either deciding the other.
///
/// This mapping is the only place the two cross a gear boundary, and Pricing's
/// rules key on both from here on: the role decides where a SKU may sit, the
/// flag whether it may be sold. A mapping that dropped an unrecognised role to
/// a default, or that inferred one from the flag, would hand Pricing a fact
/// Products never stated — so the pairing is asserted whole, for all six
/// combinations, rather than sampled on the one the fixture happens to carry.
#[tokio::test]
async fn every_role_and_flag_pairing_reaches_pricing_unchanged() {
    let db = projection_harness().await;
    let mut expected = Vec::new();

    for (n, role, sellable) in [
        (0x10_u128, "offer", true),
        (0x11, "offer", false),
        (0x12, "component", true),
        (0x13, "component", false),
        (0x14, "bundle", true),
        (0x15, "bundle", false),
    ] {
        let id = Uuid::from_u128(0xb0_1e_00_00 + n);
        let mut row = sku_row(id, &format!("ROLE-{n:x}"), "published", false);
        row.sku_type = Some(role.to_owned());
        row.sellable = Some(sellable);
        insert_projection_row(&db, row).await;
        expected.push((id, role.to_owned(), sellable));
    }

    let ids: Vec<Uuid> = expected.iter().map(|(id, _, _)| *id).collect();
    let catalog = BrowseCatalogProvider::new(db);
    let ctx = crate::test_support::authed_ctx(TENANT);
    let served = catalog
        .get_skus(&ctx, &ids)
        .await
        .expect("the registry answers");

    for (id, role, sellable) in expected {
        let sku = served
            .iter()
            .find(|sku| sku.sku_id == id)
            .unwrap_or_else(|| panic!("{role}/{sellable} is served"));
        assert_eq!(sku.sku_type, role, "the role is carried, not defaulted");
        assert_eq!(sku.sellable, sellable, "the flag is carried beside it");
    }
}

async fn seed_four_states(db: &DBProvider<DbError>) {
    insert_projection_row(db, sku_row(SKU_ID, "COMP-PUB", "published", false)).await;
    insert_projection_row(db, sku_row(DEPRECATED_ID, "COMP-DEP", "deprecated", true)).await;
    insert_projection_row(db, sku_row(DRAFT_ID, "COMP-DRAFT", "draft", false)).await;
    insert_projection_row(db, sku_row(RETIRED_ID, "COMP-RET", "retired", false)).await;
}

#[tokio::test]
async fn get_skus_omits_draft_and_retired_and_serves_published_and_deprecated() {
    let db = projection_harness().await;
    seed_four_states(&db).await;
    let catalog = BrowseCatalogProvider::new(db);
    let ctx = crate::test_support::authed_ctx(TENANT);
    let found = catalog
        .get_skus(&ctx, &[SKU_ID, DEPRECATED_ID, DRAFT_ID, RETIRED_ID])
        .await
        .expect("get_skus");
    let ids: Vec<Uuid> = found.iter().map(|sku| sku.sku_id).collect();
    assert_eq!(ids, vec![SKU_ID, DEPRECATED_ID]);
    let deprecated = found
        .iter()
        .find(|sku| sku.sku_id == DEPRECATED_ID)
        .expect("deprecated head is in the serving set");
    assert_eq!(deprecated.status, "published");
    assert!(deprecated.deprecated);
}

#[tokio::test]
async fn search_skus_omits_draft_and_retired_under_the_same_prefix() {
    let db = projection_harness().await;
    seed_four_states(&db).await;
    let catalog = BrowseCatalogProvider::new(db);
    let ctx = crate::test_support::authed_ctx(TENANT);
    let page = catalog
        .search_skus(&ctx, Some("COMP"), 50, None)
        .await
        .expect("search_skus");
    let ids: Vec<Uuid> = page.items.iter().map(|sku| sku.sku_id).collect();
    assert!(
        !ids.contains(&DRAFT_ID) && !ids.contains(&RETIRED_ID),
        "browse visibility must hide draft and retired, got {ids:?}"
    );
    assert!(
        ids.contains(&SKU_ID),
        "published must be searchable: {ids:?}"
    );
    assert!(
        ids.contains(&DEPRECATED_ID),
        "deprecated must be searchable: {ids:?}"
    );
    let deprecated = page
        .items
        .iter()
        .find(|sku| sku.sku_id == DEPRECATED_ID)
        .expect("deprecated head is in the serving set");
    assert_eq!(deprecated.status, "published");
    assert!(deprecated.deprecated);
}
