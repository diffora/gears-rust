#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::{change::SkuChange, publish::SkuPublish, retire::SkuRetire};
use crate::{
    infra::{
        broker::{EventSink, SkuChanged, SkuPublished, SkuRetired},
        events,
        storage::repo,
    },
    test_support::*,
};
use bss_approval::{ApprovalError, Engine, Policy, Store, SubmitRequest};
use bss_products_sdk::models::{Lifecycle, SkuContent, SkuType};
use event_broker_sdk::TypedEvent;
use std::sync::Arc;
use toolkit_db::{Db, DbTx};
use uuid::Uuid;

async fn in_tx<T: Send + 'static>(
    db: &Db,
    mut f: impl for<'a> FnMut(
        &'a DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, ApprovalError>> + Send + 'a>,
    > + Send,
) -> Result<T, crate::api::rest::TxError> {
    db.transaction_with_retry(
        toolkit_db::secure::TxConfig::default(),
        crate::api::rest::contention_db_err,
        move |tx| {
            let future = f(tx);
            Box::pin(async move { future.await.map_err(Into::into) })
        },
    )
    .await
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One committed publish/change/retire lifecycle exercises the actual subjects and outbox"
)]
async fn subjects_publish_change_refuse_corrupt_reference_and_withdraw() {
    let (db, scope, tenant, dsn) = test_db().await;
    let handle = toolkit_db::outbox::Outbox::builder(db.db())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .unwrap()
        .queue(
            events::QUEUE_NAME,
            toolkit_db::outbox::Partitions::of(events::PARTITIONS),
        )
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .unwrap();
    let conn = db.conn().unwrap();
    let category = repo::insert_category(
        &conn,
        &scope,
        tenant,
        crate::domain::category::NewCategory {
            code: "c".into(),
            name: "C".into(),
            is_default: false,
            sort_order: 0,
        },
        at(9),
    )
    .await
    .unwrap();
    let sku = seed_rest_sku(&conn, &scope, tenant, category.id, "SKU").await;
    let mut content = SkuContent::from(&sku);
    content.r#type = SkuType::Recurring;
    repo::update_sku_draft(&conn, &scope, tenant, sku.id, sku.revision, &content, at(9))
        .await
        .unwrap();
    let base = SkuPublish {
        scope: scope.clone(),
        tenant_id: tenant,
        sink: EventSink::Interim(Arc::clone(handle.outbox())),
        actor: sku.created_by,
        now: at(9),
        usage_type: None,
    };
    let id = sku.id;
    let submit_base = base.clone();
    let published = in_tx(&db.db(), move |tx| {
        let b = submit_base.clone();
        Box::pin(async move {
            Engine::submit(
                &repo::ProductsApprovalStore {
                    scope: b.scope.clone(),
                    tenant_id: tenant,
                },
                &b,
                tx,
                SubmitRequest {
                    tenant_id: tenant,
                    ref_id: id,
                    item_ids: &[id],
                    actor: b.actor,
                    policy: &Policy {
                        default_quorum: 0,
                        overrides: std::collections::BTreeMap::default(),
                    },
                    common_effective_date: None,
                    now: b.now,
                },
            )
            .await
        })
    })
    .await
    .ok()
    .unwrap();
    assert!(published.applied);
    let got = repo::find_sku(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.lifecycle, Lifecycle::Published);
    assert_eq!(got.published_version, 1);
    assert_eq!(enqueued_event_count(&dsn, SkuPublished::TYPE_ID).await, 1);
    assert_eq!(
        repo::versions(&conn, &scope, tenant, id)
            .await
            .unwrap()
            .len(),
        1
    );
    let change = SkuChange {
        base: base.clone(),
        patch: crate::domain::sku::SkuPatch {
            gl_code: Some(Some("4012".into())),
            ..Default::default()
        },
        effective_from: utc(2026, 10, 1, 0, 0, 0).date(),
        fence_op_id: None,
    };
    in_tx(&db.db(), move |tx| {
        let s = change.clone();
        Box::pin(async move {
            Engine::submit(
                &repo::ProductsApprovalStore {
                    scope: s.base.scope.clone(),
                    tenant_id: tenant,
                },
                &s,
                tx,
                SubmitRequest {
                    tenant_id: tenant,
                    ref_id: id,
                    item_ids: &[id],
                    actor: s.base.actor,
                    policy: &Policy {
                        default_quorum: 0,
                        overrides: std::collections::BTreeMap::default(),
                    },
                    common_effective_date: Some(s.effective_from),
                    now: s.base.now,
                },
            )
            .await
        })
    })
    .await
    .ok()
    .unwrap();
    let got = repo::find_sku(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.gl_code.as_deref(), Some("4012"));
    assert_eq!(got.published_version, 2);
    assert_eq!(
        repo::version_as_of(&conn, &scope, tenant, id, utc(2026, 10, 1, 0, 0, 0).date())
            .await
            .unwrap()
            .unwrap()
            .published_version,
        2
    );
    assert_eq!(
        enqueued_event_envelope(&dsn, SkuChanged::TYPE_ID).await["changed"],
        serde_json::json!(["gl_code"])
    );
    let op = Uuid::new_v4();
    let retire = SkuRetire {
        base: base.clone(),
        fence_op_id: op,
    };
    let submitted = in_tx(&db.db(), move |tx| {
        let s = retire.clone();
        Box::pin(async move {
            repo::fence_sku(
                tx,
                &s.base.scope,
                tenant,
                id,
                repo::Fence::Retire,
                op,
                s.base.now,
            )
            .await
            .map_err(super::store_err)?;
            Engine::submit(
                &repo::ProductsApprovalStore {
                    scope: s.base.scope.clone(),
                    tenant_id: tenant,
                },
                &s,
                tx,
                SubmitRequest {
                    tenant_id: tenant,
                    ref_id: id,
                    item_ids: &[id],
                    actor: s.base.actor,
                    policy: &Policy {
                        default_quorum: 1,
                        overrides: std::collections::BTreeMap::default(),
                    },
                    common_effective_date: None,
                    now: s.base.now,
                },
            )
            .await
        })
    })
    .await
    .ok()
    .unwrap();
    let unit_id = submitted.unit.id;
    repo::reserve_reference(
        &conn,
        &scope,
        tenant,
        id,
        "pricing",
        crate::domain::references::RefKind::Price,
        Uuid::new_v4(),
        base.actor,
        at(10),
    )
    .await
    .unwrap();
    let b = base.clone();
    let failed = in_tx(&db.db(), move |tx| {
        let b = b.clone();
        Box::pin(async move {
            Engine::approve(
                &repo::ProductsApprovalStore {
                    scope: b.scope.clone(),
                    tenant_id: tenant,
                },
                &SkuRetire {
                    base: b,
                    fence_op_id: op,
                },
                tx,
                unit_id,
                Uuid::new_v4(),
                1,
                None,
                at(10),
            )
            .await
        })
    })
    .await;
    assert!(matches!(
        failed,
        Err(crate::api::rest::TxError::Refused(
            crate::domain::error::DomainError::Conflict {
                code: "SKU_REFERENCED",
                ..
            }
        ))
    ));
    assert_eq!(
        repo::find_sku(&conn, &scope, tenant, id)
            .await
            .unwrap()
            .unwrap()
            .lifecycle,
        Lifecycle::Retiring
    );
    assert_eq!(enqueued_event_count(&dsn, SkuRetired::TYPE_ID).await, 0);
    in_tx(&db.db(), move |tx| {
        let b = base.clone();
        Box::pin(async move {
            let store = repo::ProductsApprovalStore {
                scope: b.scope.clone(),
                tenant_id: tenant,
            };
            let actor = b.actor;
            Engine::withdraw(
                &store,
                &SkuRetire {
                    base: b,
                    fence_op_id: op,
                },
                tx,
                unit_id,
                actor,
                at(11),
            )
            .await?;
            assert_eq!(store.decisions(tx, unit_id).await?.len(), 0);
            Ok(())
        })
    })
    .await
    .ok()
    .unwrap();
    let got = repo::find_sku(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.lifecycle, Lifecycle::Published);
    assert!(got.pending_unit_id.is_none());
    assert!(
        got.approved_by_unit_id.is_some(),
        "withdrawal retains previous approval attribution"
    );
    handle.stop().await;
}
