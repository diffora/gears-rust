//! Native Postgres constraints through the same scoped repositories.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;
use bss_pricing::infra::storage::{
    RepoError,
    entity::{price, price_book, price_row},
    repo::{book_repo, price_repo, row_repo},
};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;
#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_unique_codes_and_row_decimal_roundtrip() {
    let pg = pg_support::Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().unwrap();
    let tenant = Uuid::new_v4();
    let scope = AccessScope::for_tenant(tenant);
    let now = time::OffsetDateTime::now_utc();
    let b = price_book::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        code: "standard".into(),
        name: "Standard".into(),
        currency: "EUR".into(),
        valid_from: None,
        valid_until: None,
        version: 1,
        created_at: now,
        updated_at: now,
    };
    book_repo::insert(&conn, &scope, b.clone()).await.unwrap();
    assert!(matches!(
        book_repo::insert(
            &conn,
            &scope,
            price_book::Model {
                id: Uuid::new_v4(),
                ..b.clone()
            }
        )
        .await,
        Err(RepoError::Conflict {
            code: "BOOK_CODE_TAKEN"
        })
    ));
    let p = price::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        book_id: b.id,
        sku_id: Uuid::new_v4(),
        charge_kind: "usage".into(),
        period: None,
        dimension_key: None,
        invoice_line_override: None,
        reservation_id: Uuid::new_v4(),
        reference_state: "confirmed".into(),
        version: 1,
        created_at: now,
        updated_at: now,
    };
    price_repo::insert(&conn, &scope, p.clone()).await.unwrap();
    assert!(matches!(
        price_repo::insert(
            &conn,
            &scope,
            price::Model {
                id: Uuid::new_v4(),
                ..p.clone()
            }
        )
        .await,
        Err(RepoError::Conflict {
            code: "PRICE_KEY_TAKEN"
        })
    ));
    let r = price_row::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        price_id: p.id,
        version_no: 1,
        dim_value: None,
        model: "per_unit".into(),
        price_json: serde_json::json!({"rate":"0.123456789"}),
        min_fee: Some(rust_decimal::Decimal::new(1234, 2)),
        eligibility: "all".into(),
        effective_from: now.date(),
        effective_to: None,
        keep_for_bound: false,
        closed_explicitly: false,
        temporary_until: None,
        paired_row_id: None,
        return_of_row_id: None,
        state: "approved".into(),
        pending_unit_id: None,
        approved_by_unit_id: None,
        note: None,
        created_by: Uuid::new_v4(),
        approved_at: None,
        version: 1,
        created_at: now,
        updated_at: now,
    };
    let got = row_repo::insert(&conn, &scope, r.clone()).await.unwrap();
    assert_eq!(got.price_json, r.price_json);
    assert_eq!(got.min_fee, r.min_fee);
    assert!(matches!(
        row_repo::insert(
            &conn,
            &scope,
            price_row::Model {
                id: Uuid::new_v4(),
                version_no: 2,
                ..r.clone()
            }
        )
        .await,
        Err(RepoError::Conflict {
            code: "WINDOW_OVERLAP"
        })
    ));
    assert!(matches!(
        row_repo::insert(
            &conn,
            &scope,
            price_row::Model {
                id: Uuid::new_v4(),
                dim_value: Some("us".into()),
                ..r
            }
        )
        .await,
        Err(RepoError::Conflict {
            code: "ROW_VERSION_TAKEN"
        })
    ));
}
