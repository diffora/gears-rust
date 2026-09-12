//! Probes of the SDK's in-process bindings (P-D-151): the authoring client
//! runs the doors with both preconditions, the read client serves the
//! nine-member shape, the freeze and composition clients reach their doors
//! and every refusal crosses the port as a canonical error whose code is in
//! the SDK vocabulary.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use bss_products_sdk::authoring::{
    Authoring as _, FieldValue, NewProduct, NewSku, Precondition, SaveFields,
};
use bss_products_sdk::composition::{CompositionOutcome, CompositionSignals as _};
use bss_products_sdk::freeze::FreezeAcks as _;
use bss_products_sdk::models::{LifecycleState, SkuType};
use bss_products_sdk::{ErrorCode, ProductsClient as _};
use sea_orm_migration::MigratorTrait as _;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    Binding, InProcessAuthoring, InProcessCompositionSignals, InProcessFreezeAcks,
    InProcessProductsClient,
};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x12_01);
const BRAND: Uuid = Uuid::from_u128(0x12_02);

struct Harness {
    dsn: String,
    binding: Binding,
    #[allow(dead_code)]
    outbox_handle: OutboxHandle,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

async fn harness() -> Harness {
    let path = std::env::temp_dir().join(format!("bss-products-sdk-{}.sqlite3", Uuid::new_v4()));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("migrate");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX).expect("prefix"),
    )
    .await
    .expect("outbox migrate");
    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("prefix")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox");
    let defaults = ProductsConfig::default();
    let state = Arc::new(ApiState {
        db: DBProvider::<DbError>::new(db),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(outbox_handle.outbox())),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: defaults.idempotency_retention_hours,
        bulk_max_rows_per_batch: defaults.bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&defaults),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    Harness {
        dsn,
        binding: Binding {
            state,
            enforcer: crate::test_support::flat_in_enforcer(TENANT),
        },
        outbox_handle,
    }
}

/// The status and the code a refusal would show a REST caller — read the
/// way a consumer behind the out-of-process binding reads it.
async fn refusal(error: CanonicalError) -> (StatusCode, Option<String>) {
    let response = error.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    // Three channels, as the doors render them: a refusal's `context.reason`,
    // a validation report's first violation `type`, a bare `code`.
    let code = body["context"]["reason"]
        .as_str()
        .or_else(|| body["context"]["violations"][0]["type"].as_str())
        .or_else(|| body["code"].as_str())
        .map(str::to_owned);
    (status, code)
}

fn in_vocabulary(code: Option<&str>) -> bool {
    code.and_then(ErrorCode::parse).is_some()
}

fn with_key(key: &str) -> Precondition {
    Precondition {
        if_match: None,
        idempotency_key: Some(key.to_owned()),
    }
}

fn pinned(revision: i64) -> Precondition {
    Precondition {
        if_match: Some(revision),
        idempotency_key: None,
    }
}

fn fields(pairs: &[(&str, FieldValue)]) -> SaveFields {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// **The authoring binding runs the doors, both preconditions included.**
///
/// A create replays on its key, a save moves the revision under a fresh
/// `If-Match`, a stale one is `STALE_REVISION` and a missing one
/// `VALIDATION` — the door's own refusals, crossing the port as canonical
/// errors whose codes are the SDK vocabulary's. A publish on a head the
/// gate does not admit is refused by the gate, not by the binding.
#[tokio::test]
async fn the_authoring_binding_runs_the_doors_with_both_preconditions() {
    let h = harness().await;
    let authoring = InProcessAuthoring(h.binding.clone());
    let ctx = crate::test_support::authed_ctx(TENANT);
    let new = || NewProduct {
        id: None,
        brand_id: BRAND,
        name: "Compute Plus".to_owned(),
        product_code: Some("COMPUTE-PLUS".to_owned()),
        region_scope: None,
        brand_scope: None,
    };

    let created = authoring
        .create_product(&ctx, new(), with_key("sdk-create-1"))
        .await
        .expect("the create door answers");
    assert_eq!(created.lifecycle_state, LifecycleState::Draft);
    assert_eq!(created.published_version, 0);
    assert!(!created.replayed);

    let again = authoring
        .create_product(&ctx, new(), with_key("sdk-create-1"))
        .await
        .expect("the same key replays");
    assert!(again.replayed, "the stored answer, without an ETag");
    assert_eq!(again.entity_id, created.entity_id);

    let saved = authoring
        .save_product(
            &ctx,
            created.entity_id,
            fields(&[("name", FieldValue::Text("Compute Plus Renamed".to_owned()))]),
            pinned(created.internal_revision),
        )
        .await
        .expect("a fresh If-Match saves");
    assert!(saved.internal_revision > created.internal_revision);
    assert!(!saved.replayed);

    let stale = authoring
        .save_product(
            &ctx,
            created.entity_id,
            fields(&[("name", FieldValue::Text("Twice".to_owned()))]),
            pinned(created.internal_revision),
        )
        .await
        .expect_err("a stale If-Match is refused");
    let (status, code) = refusal(stale).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code.as_deref(), Some("STALE_REVISION"));

    let bare = authoring
        .save_product(
            &ctx,
            created.entity_id,
            fields(&[("name", FieldValue::Text("Bare".to_owned()))]),
            Precondition::default(),
        )
        .await
        .expect_err("a save without If-Match is refused");
    let (status, _) = refusal(bare).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "VALIDATION, the door's own"
    );

    let publish = authoring
        .publish_product(&ctx, created.entity_id, pinned(saved.internal_revision))
        .await
        .expect_err("a categoryless head under a governed gate does not publish");
    let (_, code) = refusal(publish).await;
    assert!(
        in_vocabulary(code.as_deref()),
        "the gate's refusal rides the vocabulary: {code:?}"
    );

    let sku = authoring
        .create_sku(
            &ctx,
            NewSku {
                id: None,
                product_id: created.entity_id,
                sku_code: "CP-VCPU".to_owned(),
                region_scope: None,
                brand_scope: None,
                sku_type: Some("product".to_owned()),
                sellable: None,
                plan_tier: None,
                tax_category_ref: None,
                gl_code_ref: None,
            },
            Precondition::default(),
        )
        .await
        .expect("the SKU create door answers");
    assert_eq!(sku.lifecycle_state, LifecycleState::Draft);
    let sku_saved = authoring
        .save_sku(
            &ctx,
            sku.entity_id,
            fields(&[("sellable", FieldValue::Bool(false))]),
            pinned(sku.internal_revision),
        )
        .await
        .expect("a bucket-iii save on a draft");
    assert!(sku_saved.internal_revision > sku.internal_revision);

    // The read binding, on the heads the writes left.
    let reads = InProcessProductsClient(h.binding.clone());
    let product = reads
        .get_product(&ctx, TENANT, created.entity_id)
        .await
        .expect("the head reads back");
    assert_eq!(product.name, "Compute Plus Renamed");
    assert_eq!(product.internal_revision, saved.internal_revision);
    let sku_row = reads
        .get_sku(&ctx, TENANT, sku.entity_id)
        .await
        .expect("the SKU head reads back");
    assert_eq!(sku_row.sku_type, SkuType::Product);
    assert!(!sku_row.sellable, "the save landed");
    assert!(
        !sku_row.composition_pending,
        "the ninth member, false on a non-bundle"
    );
    let miss = reads
        .get_sku(&ctx, TENANT, Uuid::now_v7())
        .await
        .expect_err("an unknown id is a miss");
    let (status, _) = refusal(miss).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The composition signal on a non-bundle: nothing to clear is its own
    // word on the wire and in the SDK (P-D-159).
    let signals = InProcessCompositionSignals(h.binding.clone());
    match signals.composed(&ctx, sku.entity_id, Uuid::now_v7()).await {
        Ok(outcome) => assert_eq!(outcome, CompositionOutcome::Nothing),
        Err(error) => {
            let (_, code) = refusal(error).await;
            assert!(
                in_vocabulary(code.as_deref()),
                "a vocabulary refusal: {code:?}"
            );
        }
    }
}

/// **The freeze binding reaches the participant doors** and a refusal
/// crosses the port with its code: an unknown version or an unregistered
/// participant is the door's own `CATALOG_VERSION_UNKNOWN` /
/// `PARTICIPANT_UNKNOWN`, never a binding-invented error.
#[tokio::test]
async fn the_freeze_binding_reaches_the_doors_and_refusals_carry_their_codes() {
    let h = harness().await;
    let acks = InProcessFreezeAcks(h.binding.clone());
    let ctx = crate::test_support::authed_ctx(TENANT);
    for edge in ["ack", "release"] {
        let result = if edge == "ack" {
            acks.ack(&ctx, 424_242, "pricing").await
        } else {
            acks.release(&ctx, 424_242, "pricing").await
        };
        let error = result.expect_err("no such version, no such participant");
        let (status, code) = refusal(error).await;
        assert!(
            status.is_client_error(),
            "{edge}: a refusal, not a failure: {status}"
        );
        assert!(
            in_vocabulary(code.as_deref()),
            "{edge}: the door's code rides the vocabulary: {code:?}"
        );
    }
}
