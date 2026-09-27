#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::infra::storage::repo::{AuditCommon, LifecycleMove, write_eventless_act_audit};
use crate::test_support::{test_db, utc};
use sea_orm::Set;
use toolkit_db::secure::SecureInsertExt;
use toolkit_odata::CursorV1;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const SKU: Uuid = Uuid::from_u128(0x5c_01);
const OTHER_SKU: Uuid = Uuid::from_u128(0x5c_02);
const UNIT: Uuid = Uuid::from_u128(0x0a_01);
const OTHER_UNIT: Uuid = Uuid::from_u128(0x0a_02);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);

async fn unit(runner: &impl DBRunner, id: Uuid, tenant: Uuid, sku: Uuid, kind: &str) {
    let model = approval_unit::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant),
        kind: Set(kind.to_owned()),
        ref_type: Set("sku".to_owned()),
        ref_id: Set(sku),
        state: Set("pending".to_owned()),
        common_effective_date: Set(None),
        quorum_required: Set(1),
        generation: Set(1),
        submitted_by: Set(ACTOR),
        submitted_at: Set(utc(2026, 9, 27, 0, 0, 0)),
        decided_at: Set(None),
        decided_note: Set(None),
        snapshot: Set(serde_json::json!({})),
        snapshot_hash: Set("h".to_owned()),
        version: Set(1),
    };
    approval_unit::Entity::insert(model.clone())
        .secure()
        .scope_with_model(&AccessScope::for_tenant(tenant), &model)
        .unwrap()
        .exec(runner)
        .await
        .unwrap();
}

/// One audit row: `n` names it (its audit id is `n`), at `second` past midnight.
#[allow(
    clippy::too_many_arguments,
    reason = "each seeded row spells its whole identity"
)]
async fn row(
    runner: &impl DBRunner,
    n: u128,
    tenant: Uuid,
    kind: &str,
    subject: Uuid,
    second: u8,
    lifecycle: LifecycleMove,
    note: Option<&str>,
) {
    write_eventless_act_audit(
        runner,
        &AccessScope::for_tenant(tenant),
        AuditCommon {
            audit_id: Uuid::from_u128(n),
            tenant_id: tenant,
            actor_ref: ACTOR,
            action: format!("act.{n}"),
            subject_kind: kind.to_owned(),
            reason: note.map(ToOwned::to_owned),
            correlation_id: None,
            written_at: utc(2026, 9, 27, 10, 0, second),
            lifecycle,
        },
        subject,
        None,
    )
    .await
    .unwrap();
}

/// Every page of the history at `limit`, followed by its cursor, as `act.<n>` names.
async fn walk(runner: &impl DBRunner, limit: u64) -> Vec<String> {
    let mut names = Vec::new();
    let mut query = ODataQuery::default().with_limit(limit);
    loop {
        let page = page_sku_history(runner, TENANT, SKU, &query).await.unwrap();
        names.extend(page.items.iter().map(|e| e.action.clone()));
        let Some(next) = page.page_info.next_cursor else {
            break;
        };
        query = ODataQuery::default()
            .with_limit(limit)
            .with_cursor(CursorV1::decode(&next).unwrap());
        assert!(names.len() < 50, "the walk ends: {names:?}");
    }
    names
}

/// The history is the SKU's own rows and its units' rows, in the tenant, ordered by
/// `(written_at, audit_id)` whatever the order they were written in; a page boundary inside a tie
/// of `written_at` skips and repeats nothing; a unit row names its unit and kind; a row without the
/// lifecycle columns reads none.
#[tokio::test]
async fn the_history_orders_by_written_at_then_audit_id_across_every_page() {
    let (db, _, _, _) = test_db().await;
    let conn = db.conn().unwrap();
    unit(&conn, UNIT, TENANT, SKU, "sku_retire").await;
    unit(&conn, OTHER_UNIT, TENANT, OTHER_SKU, "sku_publish").await;
    let moved = LifecycleMove::between(Lifecycle::Published, Lifecycle::Retiring);
    // Three rows share one instant, written against their id order; one is earlier, one later.
    row(&conn, 3, TENANT, "sku", SKU, 5, moved, None).await;
    row(
        &conn,
        1,
        TENANT,
        "approval_unit",
        UNIT,
        5,
        moved,
        Some("n1"),
    )
    .await;
    row(&conn, 2, TENANT, "sku", SKU, 5, moved, None).await;
    row(&conn, 9, TENANT, "sku", SKU, 1, LifecycleMove::NONE, None).await;
    row(&conn, 4, TENANT, "approval_unit", UNIT, 7, moved, None).await;
    // Not the SKU's: another SKU, another SKU's unit, another tenant, a category.
    row(&conn, 20, TENANT, "sku", OTHER_SKU, 5, moved, None).await;
    row(
        &conn,
        21,
        TENANT,
        "approval_unit",
        OTHER_UNIT,
        5,
        moved,
        None,
    )
    .await;
    row(
        &conn,
        22,
        Uuid::from_u128(0x7e_12),
        "sku",
        SKU,
        5,
        moved,
        None,
    )
    .await;
    row(
        &conn,
        23,
        TENANT,
        "category",
        SKU,
        5,
        LifecycleMove::NONE,
        None,
    )
    .await;

    let expected = ["act.9", "act.1", "act.2", "act.3", "act.4"];
    for limit in [1, 2, 3, 200] {
        assert_eq!(walk(&conn, limit).await, expected, "limit {limit}");
    }
    let page = page_sku_history(&conn, TENANT, SKU, &ODataQuery::default())
        .await
        .unwrap();
    assert_eq!(page.page_info.limit, 50, "the default page");
    let first = &page.items[0];
    assert_eq!(
        (first.from_lifecycle, first.to_lifecycle, first.unit_id),
        (None, None, None),
        "a row without the columns reads none"
    );
    let unit_row = &page.items[1];
    assert_eq!(
        (
            unit_row.unit_id,
            unit_row.unit_kind.as_deref(),
            unit_row.note.as_deref(),
            unit_row.from_lifecycle,
            unit_row.to_lifecycle,
            unit_row.actor,
            unit_row.at,
        ),
        (
            Some(UNIT),
            Some("sku_retire"),
            Some("n1"),
            Some(Lifecycle::Published),
            Some(Lifecycle::Retiring),
            ACTOR,
            utc(2026, 9, 27, 10, 0, 5),
        )
    );
    assert_eq!(page.items[2].unit_kind, None, "a SKU row names no unit");
    let clamped = page_sku_history(&conn, TENANT, SKU, &ODataQuery::default().with_limit(5000))
        .await
        .unwrap();
    assert_eq!(clamped.page_info.limit, 200, "`$top` is clamped at 200");
}
