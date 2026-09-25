//! Real scoped repository contracts, including two connections to one `SQLite` file.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_pricing::{
    domain::price::OpState,
    infra::storage::{
        RepoError,
        entity::{price, price_book, price_row, reference_op},
        repo::{book_repo, price_repo, reference_op_repo, row_repo},
    },
};
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_db::{ConnectOpts, Db, DbError};
use uuid::Uuid;
mod storage_support;
use storage_support::{at, test_db};
fn book(tenant: Uuid) -> price_book::Model {
    price_book::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant,
        code: "standard".into(),
        name: "Standard".into(),
        currency: "EUR".into(),
        valid_from: None,
        valid_until: None,
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    }
}
fn price(b: &price_book::Model) -> price::Model {
    price::Model {
        id: Uuid::new_v4(),
        tenant_id: b.tenant_id,
        book_id: b.id,
        sku_id: Uuid::new_v4(),
        charge_kind: "usage".into(),
        period: None,
        dimension_key: None,
        invoice_line_override: None,
        reservation_id: Uuid::new_v4(),
        reference_state: "confirmed".into(),
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    }
}
fn row(p: &price::Model) -> price_row::Model {
    price_row::Model {
        id: Uuid::new_v4(),
        tenant_id: p.tenant_id,
        price_id: p.id,
        version_no: 1,
        dim_value: None,
        model: "per_unit".into(),
        price_json: serde_json::json!({"rate":"0.1"}),
        min_fee: Some("12.34".into()),
        eligibility: "all".into(),
        effective_from: at(9).date(),
        effective_to: None,
        keep_for_bound: false,
        closed_explicitly: false,
        temporary_until: None,
        paired_row_id: None,
        return_of_row_id: None,
        state: "draft".into(),
        pending_unit_id: None,
        approved_by_unit_id: None,
        note: None,
        created_by: Uuid::new_v4(),
        approved_at: None,
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    }
}
fn op(tenant: Uuid, state: OpState, when: OffsetDateTime) -> reference_op::Model {
    reference_op::Model {
        op_id: Uuid::new_v4(),
        tenant_id: tenant,
        kind: "create_price".into(),
        price_id: Uuid::new_v4(),
        sku_id: Uuid::new_v4(),
        reservation_id: None,
        idempotency_key: Some("key".into()),
        state: state.as_str().into(),
        outcome: None,
        attempts: 0,
        next_attempt_at: when,
        last_error: None,
        created_by: Uuid::new_v4(),
        created_at: at(9),
        updated_at: at(9),
    }
}
#[tokio::test]
async fn matrix_2_book_code_unique_names_are_not() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book(tenant);
    book_repo::insert(&conn, &scope, b.clone()).await.unwrap();
    let error = book_repo::insert(
        &conn,
        &scope,
        price_book::Model {
            id: Uuid::new_v4(),
            ..b.clone()
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        RepoError::Conflict {
            code: "BOOK_CODE_TAKEN"
        }
    ));
    book_repo::insert(
        &conn,
        &scope,
        price_book::Model {
            id: Uuid::new_v4(),
            code: "other".into(),
            ..b
        },
    )
    .await
    .unwrap();
    assert_eq!(
        book_repo::list(&conn, &scope, tenant).await.unwrap().len(),
        2
    );
}
#[tokio::test]
async fn matrix_4_price_key_coalesces_null_period() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let p = price(&b);
    price_repo::insert(&conn, &scope, p.clone()).await.unwrap();
    let err = price_repo::insert(
        &conn,
        &scope,
        price::Model {
            id: Uuid::new_v4(),
            ..p
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        RepoError::Conflict {
            code: "PRICE_KEY_TAKEN"
        }
    ));
}
#[tokio::test]
async fn tenant_scope_and_parent_ownership_are_enforced() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let other = Uuid::new_v4();
    let foreign = AccessScope::for_tenant(other);
    assert!(
        book_repo::find(&conn, &foreign, tenant, b.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        book_repo::insert(&conn, &foreign, book(tenant))
            .await
            .is_err()
    );
    let mut p = price(&b);
    p.tenant_id = other;
    assert!(matches!(
        price_repo::insert(&conn, &foreign, p).await,
        Err(RepoError::Conflict {
            code: "BOOK_NOT_FOUND"
        })
    ));
    let p = price_repo::insert(&conn, &scope, price(&b)).await.unwrap();
    let mut r = row(&p);
    r.tenant_id = other;
    assert!(matches!(
        row_repo::insert(&conn, &foreign, r).await,
        Err(RepoError::Conflict {
            code: "PRICE_NOT_FOUND"
        })
    ));
}
#[tokio::test]
async fn version_guard_and_row_roundtrip() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let mut changed = b.clone();
    changed.name = "New name".into();
    book_repo::update(&conn, &scope, changed.clone())
        .await
        .unwrap();
    assert!(matches!(
        book_repo::update(&conn, &scope, changed).await,
        Err(RepoError::Conflict {
            code: "STALE_REVISION"
        })
    ));
    let p = price_repo::insert(&conn, &scope, price(&b)).await.unwrap();
    let r = row(&p);
    assert_eq!(row_repo::insert(&conn, &scope, r.clone()).await.unwrap(), r);
    assert_eq!(
        row_repo::find(&conn, &scope, tenant, r.id).await.unwrap(),
        Some(r.clone())
    );
    assert_eq!(
        row_repo::for_price(&conn, &scope, tenant, p.id)
            .await
            .unwrap(),
        vec![r.clone()]
    );
    let mut changed = r.clone();
    changed.note = Some("edit".into());
    row_repo::update_draft(&conn, &scope, changed.clone())
        .await
        .unwrap();
    assert!(
        row_repo::update_draft(&conn, &scope, changed)
            .await
            .is_err()
    );
    row_repo::delete_draft(&conn, &scope, tenant, r.id, 2)
        .await
        .unwrap();
    assert!(
        row_repo::find(&conn, &scope, tenant, r.id)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn approved_start_unique_per_chain_and_approved_money_cannot_edit() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let p = price_repo::insert(&conn, &scope, price(&b)).await.unwrap();
    let mut r = row(&p);
    r.state = "approved".into();
    row_repo::insert(&conn, &scope, r.clone()).await.unwrap();
    let mut next = r.clone();
    next.id = Uuid::new_v4();
    next.version_no = 2;
    assert!(matches!(
        row_repo::insert(&conn, &scope, next.clone()).await,
        Err(RepoError::Conflict {
            code: "WINDOW_OVERLAP"
        })
    ));
    next.dim_value = Some("us".into());
    row_repo::insert(&conn, &scope, next).await.unwrap();
    assert!(
        row_repo::update_draft(&conn, &scope, r.clone())
            .await
            .is_err()
    );
    assert!(
        row_repo::delete_draft(&conn, &scope, tenant, r.id, 1)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn reference_op_due_is_scoped_ordered_bounded_and_survives_absent_price() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let a = op(tenant, OpState::Reserving, at(8));
    let b = op(tenant, OpState::Written, at(9));
    for m in [
        a.clone(),
        b.clone(),
        op(tenant, OpState::Done, at(7)),
        op(tenant, OpState::Releasing, at(11)),
    ] {
        reference_op_repo::insert(&conn, &scope, m).await.unwrap();
    }
    let foreign = Uuid::new_v4();
    reference_op_repo::insert(
        &conn,
        &AccessScope::for_tenant(foreign),
        op(foreign, OpState::Reserving, at(6)),
    )
    .await
    .unwrap();
    assert_eq!(
        reference_op_repo::due(&conn, &scope, at(10), 1)
            .await
            .unwrap(),
        vec![a.clone()]
    );
    assert_eq!(
        reference_op_repo::due(&conn, &scope, at(10), 10)
            .await
            .unwrap(),
        vec![a, b]
    );
}
#[tokio::test]
async fn reference_op_transition_is_conditional_and_persists_retry_fields() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let m = op(tenant, OpState::Reserving, at(9));
    reference_op_repo::insert(&conn, &scope, m.clone())
        .await
        .unwrap();
    let fields = reference_op_repo::TransitionFields {
        reservation_id: Some(Uuid::new_v4()),
        outcome: Some("refused".into()),
        attempts: 3,
        next_attempt_at: at(11),
        last_error: Some("transient".into()),
        updated_at: at(10),
    };
    reference_op_repo::transition(
        &conn,
        &scope,
        m.op_id,
        OpState::Reserving,
        OpState::Cancelling,
        &fields,
    )
    .await
    .unwrap();
    assert!(matches!(
        reference_op_repo::transition(
            &conn,
            &scope,
            m.op_id,
            OpState::Reserving,
            OpState::Written,
            &fields
        )
        .await,
        Err(RepoError::Conflict {
            code: "REFERENCE_OP_CONTENDED"
        })
    ));
    let got = reference_op_repo::find(&conn, &scope, tenant, m.op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.state, "cancelling");
    assert_eq!(got.attempts, 3);
    assert_eq!(got.reservation_id, fields.reservation_id);
    assert_eq!(got.last_error, fields.last_error);
    assert_eq!(got.outcome, fields.outcome);
}
fn db_error(e: &RepoError) -> Option<&sea_orm::DbErr> {
    match e {
        RepoError::Driver { source, .. } => Some(source),
        _ => None,
    }
}
async fn writer(
    db: Db,
    scope: AccessScope,
    id: Uuid,
    barrier: Arc<tokio::sync::Barrier>,
) -> Result<(), RepoError> {
    let mut attempt = 0;
    db.transaction_with_retry(TxConfig::serializable(), db_error, move |tx| {
        attempt += 1;
        let first = attempt == 1;
        let scope = scope.clone();
        let barrier = Arc::clone(&barrier);
        Box::pin(async move {
            if first {
                barrier.wait().await;
            }
            reference_op_repo::transition(
                tx,
                &scope,
                id,
                OpState::Reserving,
                OpState::Written,
                &reference_op_repo::TransitionFields {
                    reservation_id: Some(Uuid::new_v4()),
                    outcome: None,
                    attempts: 1,
                    next_attempt_at: at(10),
                    last_error: None,
                    updated_at: at(10),
                },
            )
            .await
        })
    })
    .await
}
#[tokio::test]
async fn two_real_writers_one_sqlite_file_only_one_transition_wins() {
    let (db, scope, tenant, dsn) = test_db().await;
    let other = toolkit_db::connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..ConnectOpts::default()
        },
    )
    .await
    .unwrap();
    let m = op(tenant, OpState::Reserving, at(9));
    reference_op_repo::insert(&db.conn().unwrap(), &scope, m.clone())
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (a, b) = tokio::join!(
        writer(db.db(), scope.clone(), m.op_id, Arc::clone(&barrier)),
        writer(other, scope.clone(), m.op_id, barrier)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let loser = if let Err(e) = a { e } else { b.unwrap_err() };
    assert!(matches!(
        loser,
        RepoError::Conflict {
            code: "REFERENCE_OP_CONTENDED"
        }
    ));
    assert_eq!(
        reference_op_repo::find(&db.conn().unwrap(), &scope, tenant, m.op_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "written"
    );
}
#[tokio::test]
async fn op_and_price_writes_roll_back_atomically() {
    let (db, scope, tenant, _) = test_db().await;
    let m = op(tenant, OpState::Reserving, at(9));
    let id = m.op_id;
    let txscope = scope.clone();
    let result: Result<(), RepoError> = db
        .db()
        .transaction_with_retry(TxConfig::serializable(), db_error, move |tx| {
            let scope = txscope.clone();
            let m = m.clone();
            Box::pin(async move {
                reference_op_repo::insert(tx, &scope, m).await?;
                Err(RepoError::Db("rollback probe".into()))
            })
        })
        .await;
    assert!(result.is_err());
    assert!(
        reference_op_repo::find(&db.conn().unwrap(), &scope, tenant, id)
            .await
            .unwrap()
            .is_none()
    );
}
#[test]
fn serializable_transaction_errors_keep_the_driver_variant() {
    let original = sea_orm::DbErr::Exec(sea_orm::RuntimeErr::Internal("locked".into()));
    let wrapped = RepoError::from(DbError::Sea(original));
    assert!(matches!(wrapped.to_db_err(), sea_orm::DbErr::Exec(_)));
}
#[test]
fn unique_messages_match_both_engines() {
    use bss_pricing::infra::storage::repo::unique_code;
    for (message, code) in [
        (
            "duplicate key value violates unique constraint pricing_price_book_tenant_id_code_key",
            "BOOK_CODE_TAKEN",
        ),
        (
            "UNIQUE constraint failed: pricing_price_book.tenant_id, pricing_price_book.code",
            "BOOK_CODE_TAKEN",
        ),
        (
            "duplicate key value violates unique constraint pricing_price_key",
            "PRICE_KEY_TAKEN",
        ),
        (
            "UNIQUE constraint failed: index 'pricing_price_key'",
            "PRICE_KEY_TAKEN",
        ),
        (
            "duplicate key value violates unique constraint pricing_price_row_approved_start",
            "WINDOW_OVERLAP",
        ),
        (
            "UNIQUE constraint failed: index 'pricing_price_row_approved_start'",
            "WINDOW_OVERLAP",
        ),
        (
            "duplicate key value violates unique constraint pricing_price_row_price_id_version_no_key",
            "ROW_VERSION_TAKEN",
        ),
        (
            "UNIQUE constraint failed: pricing_price_row.price_id, pricing_price_row.version_no",
            "ROW_VERSION_TAKEN",
        ),
    ] {
        assert_eq!(unique_code(message), Some(code));
    }
    assert_eq!(unique_code("unrelated constraint"), None);
}
#[tokio::test]
async fn settings_and_dimensions_roundtrip_and_version_guards() {
    use bss_pricing::infra::storage::{
        entity::{dimension_key, settings},
        repo::{dimension_repo, settings_repo},
    };
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let s = settings::Model {
        tenant_id: tenant,
        default_timing: "arrears".into(),
        default_rounding: "half-up-2".into(),
        default_gl: None,
        default_tax_category: None,
        invoice_line_templates: serde_json::json!({"usage":"{sku}"}),
        version: 1,
        created_at: at(9),
        updated_at: at(9),
    };
    assert_eq!(
        settings_repo::insert(&conn, &scope, s.clone())
            .await
            .unwrap(),
        s
    );
    let mut changed = s.clone();
    changed.default_timing = "advance".into();
    settings_repo::update(&conn, &scope, changed.clone())
        .await
        .unwrap();
    assert!(settings_repo::update(&conn, &scope, changed).await.is_err());
    assert_eq!(
        settings_repo::find(&conn, &scope, tenant, tenant)
            .await
            .unwrap()
            .unwrap()
            .default_timing,
        "advance"
    );
    let d = dimension_key::Model {
        tenant_id: tenant,
        key: "region".into(),
        values: serde_json::json!(["eu", "us"]),
        version: 1,
    };
    assert_eq!(
        dimension_repo::insert(&conn, &scope, d.clone())
            .await
            .unwrap(),
        d
    );
    assert_eq!(
        dimension_repo::list(&conn, &scope, tenant).await.unwrap(),
        vec![d.clone()]
    );
    let mut changed = d.clone();
    changed.values = serde_json::json!(["eu", "us", "ap"]);
    dimension_repo::update(&conn, &scope, changed.clone())
        .await
        .unwrap();
    assert!(
        dimension_repo::update(&conn, &scope, changed)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn audit_and_idempotency_roll_back_with_mutation() {
    use bss_pricing::infra::storage::entity::audit_log;
    use sea_orm::{ColumnTrait, Condition, EntityTrait};
    use toolkit_db::secure::SecureEntityExt;

    use bss_pricing::infra::storage::repo::{
        audit_repo::{AuditCommon, write_eventless_act_audit},
        idempotency_repo::{claim_idempotency_key, lookup_idempotency_key},
    };
    let (db, scope, tenant, _) = test_db().await;
    let txscope = scope.clone();
    let b = book(tenant);
    let id = b.id;
    let result: Result<(), RepoError> = row_repo::transaction(&db.db(), move |tx| {
        let scope = txscope.clone();
        let b = b.clone();
        Box::pin(async move {
            claim_idempotency_key(
                tx,
                &scope,
                tenant,
                "/books",
                "key",
                b"digest",
                at(9),
                at(10),
            )
            .await?;
            book_repo::insert(tx, &scope, b).await?;
            write_eventless_act_audit(
                tx,
                &scope,
                AuditCommon {
                    audit_id: Uuid::new_v4(),
                    tenant_id: tenant,
                    actor_ref: Uuid::new_v4(),
                    action: "book.create".into(),
                    subject_kind: "price_book".into(),
                    reason: None,
                    correlation_id: None,
                    written_at: at(9),
                },
                id,
                Some(1),
            )
            .await?;
            Err(RepoError::Db("rollback probe".into()))
        })
    })
    .await;
    assert!(result.is_err());
    assert!(
        book_repo::find(&db.conn().unwrap(), &scope, tenant, id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        lookup_idempotency_key(&db.conn().unwrap(), &scope, tenant, "/books", "key", at(9))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        audit_log::Entity::find()
            .secure()
            .scope_with(&scope)
            .filter(Condition::all().add(audit_log::Column::TenantId.eq(tenant)))
            .all(&db.conn().unwrap())
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn idempotency_lookup_and_release_contract() {
    use bss_pricing::infra::storage::repo::idempotency_repo::*;
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    assert!(
        lookup_idempotency_key(&conn, &scope, tenant, "/prices", "k", at(9))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        claim_idempotency_key(&conn, &scope, tenant, "/prices", "k", b"a", at(9), at(11))
            .await
            .unwrap(),
        IdempotencyClaim::Claimed
    );
    assert!(matches!(
        lookup_idempotency_key(&conn, &scope, tenant, "/prices", "k", at(10))
            .await
            .unwrap(),
        Some(IdempotencyClaim::InFlight { .. })
    ));
    assert_eq!(
        release_idempotency_claim(&conn, &scope, tenant, "/prices", "k")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        release_idempotency_claim(&conn, &scope, tenant, "/prices", "k")
            .await
            .unwrap(),
        0
    );
    claim_idempotency_key(&conn, &scope, tenant, "/prices", "k", b"a", at(9), at(11))
        .await
        .unwrap();
    answer_idempotency_key(
        &conn,
        &scope,
        tenant,
        "/prices",
        "k",
        201,
        serde_json::json!({"id":"receipt"}),
    )
    .await
    .unwrap();
    assert_eq!(
        release_idempotency_claim(&conn, &scope, tenant, "/prices", "k")
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        lookup_idempotency_key(&conn, &scope, tenant, "/prices", "k", at(10))
            .await
            .unwrap(),
        Some(IdempotencyClaim::Answered {
            response_status: 201,
            ..
        })
    ));
}
#[tokio::test]
async fn price_receipt_updates_and_deletion_are_version_guarded() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let mut p = price(&b);
    p.reference_state = "confirmation_pending".into();
    let p = price_repo::insert(&conn, &scope, p).await.unwrap();
    price_repo::set_reference(
        &conn,
        &scope,
        tenant,
        p.id,
        1,
        bss_pricing::domain::price::ReferenceState::Confirmed,
        p.reservation_id,
        at(10),
    )
    .await
    .unwrap();
    assert!(
        price_repo::set_reference(
            &conn,
            &scope,
            tenant,
            p.id,
            1,
            bss_pricing::domain::price::ReferenceState::Lost,
            p.reservation_id,
            at(10)
        )
        .await
        .is_err()
    );
    assert_eq!(
        price_repo::find(&conn, &scope, tenant, p.id)
            .await
            .unwrap()
            .unwrap()
            .reference_state,
        "confirmed"
    );
    let txscope = scope.clone();
    let m = reference_op::Model {
        kind: "delete_price".into(),
        price_id: p.id,
        reservation_id: Some(p.reservation_id),
        ..op(tenant, OpState::Releasing, at(10))
    };
    let op_id = m.op_id;
    row_repo::transaction(&db.db(), move |tx| {
        let scope = txscope.clone();
        let m = m.clone();
        Box::pin(async move {
            price_repo::delete_empty(tx, &scope, tenant, p.id, 2).await?;
            reference_op_repo::insert(tx, &scope, m).await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    assert!(
        price_repo::find(&conn, &scope, tenant, p.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reference_op_repo::find(&conn, &scope, tenant, op_id)
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test]
async fn row_pending_ownership_is_a_conditional_versioned_write() {
    use bss_approval::{Store, Unit, UnitState};
    use bss_pricing::infra::storage::repo::approval_repo::PricingApprovalStore;
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book_repo::insert(&conn, &scope, book(tenant))
        .await
        .unwrap();
    let p = price_repo::insert(&conn, &scope, price(&b)).await.unwrap();
    let r = row_repo::insert(&conn, &scope, row(&p)).await.unwrap();
    let id = r.id;
    let unit = Uuid::new_v4();
    let txscope = scope.clone();
    row_repo::transaction(&db.db(), move |tx| {
        let scope = txscope.clone();
        Box::pin(async move {
            let store = PricingApprovalStore {
                scope: scope.clone(),
                tenant_id: tenant,
            };
            store
                .insert_unit(
                    tx,
                    &Unit {
                        id: unit,
                        tenant_id: tenant,
                        kind: "price_rows".into(),
                        ref_type: "price_book".into(),
                        ref_id: b.id,
                        state: UnitState::Pending,
                        common_effective_date: None,
                        quorum_required: 1,
                        generation: 1,
                        submitted_by: Uuid::new_v4(),
                        submitted_at: at(9),
                        decided_at: None,
                        decided_note: None,
                        snapshot: serde_json::json!({}),
                        snapshot_hash: "hash".into(),
                        version: 1,
                    },
                    &[],
                )
                .await
                .map_err(|e| RepoError::Db(e.to_string()))?;
            assert!(row_repo::try_lock(tx, &scope, tenant, id, unit, 1).await?);
            assert!(!row_repo::try_lock(tx, &scope, tenant, id, unit, 1).await?);
            Ok(())
        })
    })
    .await
    .unwrap();
    let locked = row_repo::find(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(locked.state, "pending");
    assert_eq!(locked.version, 2);
    assert!(row_repo::update_draft(&conn, &scope, locked).await.is_err());
    assert!(
        row_repo::unlock(
            &conn,
            &scope,
            tenant,
            id,
            Uuid::new_v4(),
            row_repo::Unlock::Draft
        )
        .await
        .is_err()
    );
    row_repo::unlock(&conn, &scope, tenant, id, unit, row_repo::Unlock::Draft)
        .await
        .unwrap();
    let draft = row_repo::find(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.state, "draft");
    assert!(draft.pending_unit_id.is_none());
    assert_eq!(draft.version, 3);
}
#[tokio::test]
async fn min_fee_round_trips_exactly_on_sqlite() {
    // sea-orm decodes every SQLite Decimal through f64: an authored "30.00" came back
    // as "30", and digits past f64's precision were lost. A fee is money: exact text.
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book(tenant);
    book_repo::insert(&conn, &scope, b.clone()).await.unwrap();
    let p = price(&b);
    price_repo::insert(&conn, &scope, p.clone()).await.unwrap();
    for (n, text) in ["30.00", "0.10", "12345678901234567.89", "0"]
        .into_iter()
        .enumerate()
    {
        let mut r = row(&p);
        r.version_no = i32::try_from(n).unwrap() + 1;
        r.min_fee = Some(text.into());
        row_repo::insert(&conn, &scope, r.clone()).await.unwrap();
        let back = row_repo::find(&conn, &scope, tenant, r.id)
            .await
            .unwrap()
            .unwrap();
        let fee = row_repo::to_domain(&back).unwrap().min_fee.unwrap();
        assert_eq!(
            fee.to_string(),
            text,
            "min_fee {text} must read back exactly"
        );
    }
}
#[tokio::test]
async fn min_fee_column_refuses_anything_but_an_unsigned_plain_decimal() {
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let b = book(tenant);
    book_repo::insert(&conn, &scope, b.clone()).await.unwrap();
    let p = price(&b);
    price_repo::insert(&conn, &scope, p.clone()).await.unwrap();
    for (n, text) in ["-1", "1.", "1.2.3", "1e5", "", " 3"]
        .into_iter()
        .enumerate()
    {
        let mut r = row(&p);
        r.version_no = i32::try_from(n).unwrap() + 1;
        r.min_fee = Some(text.into());
        assert!(
            row_repo::insert(&conn, &scope, r).await.is_err(),
            "the column must refuse min_fee {text:?}"
        );
    }
}
#[tokio::test]
async fn audit_rows_refuse_deletion_and_edits() {
    use bss_pricing::infra::storage::repo::audit_repo::{AuditCommon, write_eventless_act_audit};
    use sea_orm::{ConnectionTrait, Database};
    let (db, scope, tenant, dsn) = test_db().await;
    write_eventless_act_audit(
        &db.conn().unwrap(),
        &scope,
        AuditCommon {
            audit_id: Uuid::new_v4(),
            tenant_id: tenant,
            actor_ref: Uuid::new_v4(),
            action: "book.create".into(),
            subject_kind: "price_book".into(),
            reason: None,
            correlation_id: Some("c".into()),
            written_at: at(9),
        },
        Uuid::new_v4(),
        Some(1),
    )
    .await
    .unwrap();
    let raw = Database::connect(&dsn).await.unwrap();
    for statement in [
        "DELETE FROM pricing_audit",
        "UPDATE pricing_audit SET action = 'forged'",
    ] {
        let refused = raw.execute_unprepared(statement).await.unwrap_err();
        assert!(refused.to_string().contains("append-only"), "{refused}");
    }
    let kept = raw
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT action FROM pricing_audit".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.try_get::<String>("", "action").unwrap(), "book.create");
}
