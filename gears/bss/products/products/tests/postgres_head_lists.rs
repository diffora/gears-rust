//! `PostgreSQL` execution of the authoring collection queries and UUID keyset cursors.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use bss_products::infra::storage::repo::{self, NewProduct, NewSku};
use toolkit_db::odata::sea_orm_filter::LimitCfg;
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use toolkit_odata::{CursorV1, ODataQuery, parse_filter_string};
use uuid::Uuid;

const LIMITS: LimitCfg = LimitCfg {
    default: 50,
    max: 200,
};

/// Unlike `SQLite`, `PostgreSQL` executes UUID comparisons and SQL LIKE natively.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn postgres_head_search_pages_and_scopes_both_collections() {
    let pg = pg_support::Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().unwrap();
    let tenant = Uuid::new_v4();
    let foreign = Uuid::new_v4();
    let scope = AccessScope::for_tenant(tenant);
    let brand = Uuid::new_v4();
    let now = time::OffsetDateTime::now_utc();
    let mut own_products = Vec::new();
    for owner in [tenant, foreign] {
        let write_scope = AccessScope::for_tenant(owner);
        for name in ["Cloud Alpha", "Cloud Beta", "Other"] {
            let product_id = Uuid::new_v4();
            repo::insert_product(
                &conn,
                &write_scope,
                NewProduct {
                    product_id,
                    tenant_id: owner,
                    brand_id: brand,
                    name: name.to_owned(),
                    name_normalized: name.to_lowercase(),
                    product_code: None,
                    region_scope: String::new(),
                    brand_scope: String::new(),
                    created_by: "fixture".to_owned(),
                    created_at: now,
                    cloned_from: None,
                    cloned_from_version: None,
                },
            )
            .await
            .unwrap();
            repo::insert_sku(
                &conn,
                &write_scope,
                NewSku {
                    sku_id: Uuid::new_v4(),
                    tenant_id: owner,
                    product_id,
                    sku_code: format!("S-{name}"),
                    region_scope: String::new(),
                    brand_scope: String::new(),
                    created_by: "fixture".to_owned(),
                    created_at: now,
                    cloned_from: None,
                    cloned_from_version: None,
                    sku_type: "offer".to_owned(),
                    sellable: true,
                    plan_tier: "standard".to_owned(),
                    metering_unit: None,
                    usage_type_ref: None,
                },
            )
            .await
            .unwrap();
            if owner == tenant {
                own_products.push(product_id);
            }
        }
    }
    let expression = "contains(name,'Cloud') and lifecycle_state eq 'draft'";
    let query = ODataQuery::new()
        .with_filter(parse_filter_string(expression).unwrap().into_expr())
        .with_filter_hash(expression.to_owned())
        .with_limit(1);
    let first = repo::list_products_page(&conn, &scope, tenant, &query, LIMITS)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].name, "Cloud Alpha");
    let cursor = CursorV1::decode(first.page_info.next_cursor.as_deref().unwrap()).unwrap();
    let second = repo::list_products_page(
        &conn,
        &scope,
        tenant,
        &query.clone().with_cursor(cursor),
        LIMITS,
    )
    .await
    .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].name, "Cloud Beta");
    assert!(second.page_info.next_cursor.is_none());
    let cursor = CursorV1::decode(second.page_info.prev_cursor.as_deref().unwrap()).unwrap();
    let back = repo::list_products_page(&conn, &scope, tenant, &query.with_cursor(cursor), LIMITS)
        .await
        .unwrap();
    assert_eq!(back.items[0].product_id, first.items[0].product_id);

    let expression = format!(
        "product_id eq {} and contains(sku_code,'Cloud') and sellable eq true",
        own_products[0]
    );
    let query =
        ODataQuery::new().with_filter(parse_filter_string(&expression).unwrap().into_expr());
    let sku_page = repo::list_skus_page(&conn, &scope, tenant, &query, LIMITS)
        .await
        .unwrap();
    assert_eq!(sku_page.items.len(), 1);
    assert_eq!(sku_page.items[0].sku_code, "S-Cloud Alpha");
    let query = ODataQuery::new().with_limit(1);
    let mut codes = Vec::new();
    let mut request = query.clone();
    loop {
        let page = repo::list_skus_page(&conn, &scope, tenant, &request, LIMITS)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].tenant_id, tenant);
        codes.push(page.items[0].sku_code.clone());
        assert!(codes.len() <= 3);
        let Some(cursor) = page.page_info.next_cursor else {
            break;
        };
        request = query
            .clone()
            .with_cursor(CursorV1::decode(&cursor).unwrap());
    }
    assert_eq!(codes, ["S-Cloud Alpha", "S-Cloud Beta", "S-Other"]);
    let denied = AccessScope::for_tenant(foreign);
    assert!(
        repo::list_products_page(&conn, &denied, tenant, &query, LIMITS)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        repo::list_skus_page(&conn, &denied, tenant, &query, LIMITS)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}
