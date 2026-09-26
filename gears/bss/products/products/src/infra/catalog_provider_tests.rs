#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::{
    domain::{category::NewCategory, sku::NewSku},
    infra::storage::repo,
    test_support::*,
};
use bss_products_sdk::models::{Lifecycle, SkuType};
use std::sync::Arc;
use time::OffsetDateTime;

#[tokio::test]
async fn both_transports_serve_the_same_published_catalog_and_pages() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let now = OffsetDateTime::now_utc();
    let cat = repo::insert_category(
        &conn,
        &scope,
        tenant,
        NewCategory {
            code: "hosting".into(),
            name: "Hosting".into(),
            is_default: true,
            sort_order: 0,
        },
        now,
    )
    .await
    .unwrap();
    let mut ids = Vec::new();
    for (code, name, lifecycle) in [
        ("A", "storage draft", Lifecycle::Draft),
        ("B", "storage", Lifecycle::Published),
        ("C", "storage legacy", Lifecycle::Deprecated),
        ("D", "storage retired", Lifecycle::Retired),
        ("E", "storage retiring", Lifecycle::Retiring),
        ("F", "O'Brien_%", Lifecycle::Published),
    ] {
        let s = repo::insert_sku(
            &conn,
            &scope,
            tenant,
            NewSku {
                code: code.into(),
                name: name.into(),
                r#type: SkuType::Usage,
                category_id: cat.id,
                description: String::new(),
                sellable: true,
                gl_code: None,
                tax_category: Some("cloud".into()),
                invoice_line_template: None,
                billing_timing: None,
                usage_type_ref: Some("storage.bytes".into()),
                unit: Some("GiB".into()),
            },
            tenant,
            now,
        )
        .await
        .unwrap();
        repo::set_lifecycle(
            &conn,
            &scope,
            tenant,
            s.id,
            &[Lifecycle::Draft],
            lifecycle,
            now,
        )
        .await
        .unwrap();
        ids.push(s.id);
    }
    let ctx = authed_ctx(tenant);
    let provider = BrowseCatalogProvider::new(db.db(), Arc::new(flat_in_enforcer(tenant)));
    let page = provider
        .search_skus(&ctx, Some("stor"), 1, None)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0],
        CatalogSku {
            sku_id: ids[1],
            sku_code: "B".into(),
            name: "storage".into(),
            metering_unit: Some("GiB".into()),
            status: "published".into(),
            plan_tier: None,
            sku_type: "usage".into(),
            sellable: true,
            usage_type_ref: Some("storage.bytes".into()),
            deprecated: false,
        }
    );
    assert_eq!(page.next_cursor.as_deref(), Some("B"));
    let all = provider.get_skus(&ctx, &ids).await.unwrap();
    assert_eq!(all.len(), 3);
    assert!(all[1].deprecated);
    assert_eq!(all[1].status, "deprecated");
    assert_eq!(
        provider.list_tax_categories(&ctx).await.unwrap(),
        vec![CatalogTaxCategory {
            code: "cloud".into(),
            display_name: "cloud".into()
        }]
    );
    let foreign = authed_ctx(Uuid::new_v4());
    assert!(provider.get_skus(&foreign, &ids).await.unwrap().is_empty());

    let (app, _state) = rest_app_on_db(
        tenant,
        crate::api::rest::browse::router,
        Arc::new(crate::infra::usage_types::UnconfiguredUsageTypes),
        "unconfigured",
        db,
    )
    .await;
    let app = app.layer(axum::Extension(ctx.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::infra::catalog_rest_client::ProductCatalogRestClient::new(
        toolkit::contract_support::runtime::config::ClientConfig::new(format!(
            "http://{}",
            listener.local_addr().unwrap()
        )),
    )
    .unwrap();
    let requests = async {
        assert_eq!(client.get_skus(&ctx, &ids).await.unwrap(), all);
        assert_eq!(
            client
                .search_skus(&ctx, Some("stor"), 1, None)
                .await
                .unwrap(),
            page
        );
        let next = client
            .search_skus(&ctx, Some("stor"), 1, page.next_cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(
            next,
            provider
                .search_skus(&ctx, Some("stor"), 1, page.next_cursor.as_deref())
                .await
                .unwrap()
        );
        assert_eq!(next.items[0].sku_code, "C");
        assert!(next.next_cursor.is_none());
        assert_eq!(
            client
                .search_skus(&ctx, Some("O'Brien_%"), 50, None)
                .await
                .unwrap(),
            provider
                .search_skus(&ctx, Some("O'Brien_%"), 50, None)
                .await
                .unwrap()
        );
        assert_eq!(
            client.list_tax_categories(&ctx).await.unwrap(),
            provider.list_tax_categories(&ctx).await.unwrap()
        );
    };
    tokio::select! {
        () = requests => {},
        result = axum::serve(listener, app) => { result.unwrap(); panic!("server ended before requests"); }
    }
}
