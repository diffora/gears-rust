#![allow(clippy::expect_used, clippy::unwrap_used)]
//! P-D-207: the usage-type picker, served by products and read as the caller.
use super::router;
use crate::infra::usage_types::{LocalDevStaticUsageTypes, UnconfiguredUsageTypes};
use crate::test_support::{
    EmptyUsageTypes, UnreachableUsageTypes, body_json, denying_collector_catalog, get,
    rest_app_with_catalog,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use bss_products_sdk::usage_types::{
    UsageTypeAnswer, UsageTypeBinding, UsageTypeCatalog, UsageTypePage,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

const PICKER: &str = "/bss-products/v1/usage-types";

/// One `list` call: `q`, `kind`, `limit`, `cursor`.
type Asked = (Option<String>, Option<String>, u32, Option<String>);
/// A catalog that records what it was asked and answers one page.
#[derive(Default)]
struct Recording {
    asked: Mutex<Vec<Asked>>,
}

#[async_trait]
impl UsageTypeCatalog for Recording {
    async fn resolve(&self, _: &SecurityContext, _: &str) -> UsageTypeAnswer {
        UsageTypeAnswer::Unavailable
    }
    async fn list(
        &self,
        _: &SecurityContext,
        q: Option<&str>,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError> {
        self.asked.lock().unwrap().push((
            q.map(ToOwned::to_owned),
            kind.map(ToOwned::to_owned),
            limit,
            cursor.map(ToOwned::to_owned),
        ));
        Ok(UsageTypePage {
            items: vec![UsageTypeBinding {
                gts_id: "gts.cf.core.uc.usage_record.v1~cf.bss.usage_type.storage.v1".into(),
                kind: "counter".into(),
                metadata_fields: vec!["region".into()],
            }],
            next_cursor: Some("next".into()),
            prev_cursor: None,
            limit: limit.min(100),
        })
    }
}

#[tokio::test]
async fn the_picker_lists_the_catalog_page_with_its_source() {
    let tenant = Uuid::new_v4();
    let catalog = Arc::new(Recording::default());
    let (app, _) = rest_app_with_catalog(tenant, router, catalog.clone(), "registry").await;
    let r = get(
        &app,
        tenant,
        &format!("{PICKER}?q=storage&kind=counter&limit=150&cursor=abc"),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        body_json(r).await,
        json!({
            "source": "registry",
            "items": [{
                "gts_id": "gts.cf.core.uc.usage_record.v1~cf.bss.usage_type.storage.v1",
                "kind": "counter",
                "metadata_fields": ["region"],
            }],
            "page_info": {"next_cursor": "next", "prev_cursor": null, "limit": 100},
        })
    );
    // No parameters: the default page of 50; a limit past 200 is clamped, never refused.
    assert_eq!(get(&app, tenant, PICKER).await.status(), StatusCode::OK);
    assert_eq!(
        get(&app, tenant, &format!("{PICKER}?limit=5000"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        *catalog.asked.lock().unwrap(),
        vec![
            (
                Some("storage".to_owned()),
                Some("counter".to_owned()),
                150,
                Some("abc".to_owned())
            ),
            (None, None, 50, None),
            (None, None, 200, None),
        ]
    );
    for bad in ["limit=0", "limit=many", "limit=-1"] {
        let r = get(&app, tenant, &format!("{PICKER}?{bad}")).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        catalog.asked.lock().unwrap().len(),
        3,
        "a refused query asks nobody"
    );
}

#[tokio::test]
async fn the_picker_tells_no_catalog_from_an_empty_or_unreachable_one() {
    let tenant = Uuid::new_v4();
    let (app, _) = rest_app_with_catalog(
        tenant,
        router,
        Arc::new(UnconfiguredUsageTypes),
        "unconfigured",
    )
    .await;
    assert_eq!(
        get(&app, tenant, PICKER).await.status(),
        StatusCode::NOT_IMPLEMENTED
    );
    let (app, _) = rest_app_with_catalog(tenant, router, Arc::new(EmptyUsageTypes), "test").await;
    let r = get(&app, tenant, PICKER).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["items"], json!([]));
    let (app, _) =
        rest_app_with_catalog(tenant, router, Arc::new(UnreachableUsageTypes), "test").await;
    assert_eq!(
        get(&app, tenant, PICKER).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (app, _) = rest_app_with_catalog(
        tenant,
        router,
        Arc::new(LocalDevStaticUsageTypes),
        "local_dev_static",
    )
    .await;
    let r = get(&app, tenant, &format!("{PICKER}?q=storage")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let b = body_json(r).await;
    assert_eq!(b["source"], "local_dev_static");
    assert_eq!(b["items"].as_array().unwrap().len(), 1, "{b}");
}

/// Read as the caller: a collector that refuses the caller answers 403, not 503.
#[tokio::test]
async fn a_collector_denial_of_the_picker_is_403() {
    let tenant = Uuid::new_v4();
    let (app, _) = rest_app_with_catalog(
        tenant,
        router,
        denying_collector_catalog(),
        "usage_collector",
    )
    .await;
    let r = get(&app, tenant, PICKER).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}
