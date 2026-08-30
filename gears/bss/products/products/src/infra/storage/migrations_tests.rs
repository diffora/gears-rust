//! The roster's own invariants. The runner applies migrations in **name**
//! order and rejects a duplicate name outright, so a duplicate would be a
//! migration that silently never runs — which is what these assertions exist
//! to catch.
//!
//! Below the roster invariants: the `products_audit_log` migration's own
//! guards, exercised against the executed `SQLite` mirror. There is no
//! repository or entity for the table yet — that lands in a later slice — so
//! these tests speak to the table through a small, test-only `SeaORM` entity
//! defined in the `audit_row` module below, scoped through the same `SecureORM`
//! wrappers a real repository would use. What they exercise is the migration
//! itself: the seam-group `CHECK`, the subject-ref `CHECK`, and the
//! append-only trigger's whitelist of exactly one admitted transition.
//!
//! Below that: the `products_identity_ref` migration's own guards, exercised
//! the identical way through a second test-only entity in the
//! `identity_ref_row` module. What they exercise is the migration itself: the
//! tombstone and seen-order `CHECK`s, and the partial unique index's
//! one-active-ref-per-principal rule — including that a fresh ref for a
//! tombstoned principal is admitted, and that the uniqueness is tenant-scoped.
//!
//! Below that: the `products_idempotency` migration's own guards, exercised
//! the identical way through a third test-only entity in the
//! `idempotency_row` module. What they exercise is the migration itself: the
//! `state` roster `CHECK`, the response-group `CHECK` tying `state` to the
//! response columns, and the composite primary key that makes the claim
//! `INSERT` a gate rather than a hint.
//!
//! Below that: the `products_product`/`products_sku` append-only head-row
//! guard (`cpt-cf-bss-products-dod-append-only-guard`), exercised through a
//! fourth and fifth test-only entity, `product_row` and `sku_row`, mirroring
//! the two migrations' own schema. What they exercise is the migration
//! itself: the lifecycle edge list, the bucket-i/bucket-iii gating, the
//! `published_version`/`internal_revision` counters and the immutable-column
//! set — one refusal probe per guarded column class, and, per this suite's
//! own rule, a positive control alongside every refusal probe that could
//! otherwise pass on a guard that refuses everything.
#![allow(clippy::expect_used)]

use sea_orm_migration::MigratorTrait;

use super::Migrator;

#[test]
fn every_migration_name_is_unique() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "a duplicate migration name is a migration that never runs"
    );
}

#[test]
fn vec_order_matches_name_order() {
    // The runner sorts by name; if the vec disagrees, the file order stops
    // describing the execution order and the chain becomes unreadable.
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

#[test]
fn the_schema_migration_sorts_first() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let first = names.first().map(String::as_str);
    assert_eq!(first, Some("m20260829_000001_create_bss_schema"));
}

/// A local, test-only `SeaORM` entity for `products_audit_log`.
///
/// Not the gear's production entity — that lands with the repository in a
/// later slice — but the schema this migration creates, scoped through the
/// same `SecureORM` wrappers a real repository would use, so the tests below
/// exercise the migration's own guards rather than a hand-rolled connection.
mod audit_row {
    use sea_orm::entity::prelude::*;
    use toolkit_db_macros::Scopable;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
    #[sea_orm(table_name = "products_audit_log")]
    #[secure(tenant_col = "tenant_id", resource_col = "audit_id", no_owner, no_type)]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub audit_id: Uuid,
        pub tenant_id: Uuid,
        pub actor_ref: Uuid,
        pub action: String,
        pub subject_kind: String,
        pub subject_id: Option<Uuid>,
        pub subject_revision: Option<i64>,
        pub error_code: Option<String>,
        pub attempted_key: Option<String>,
        pub reason: Option<String>,
        pub correlation_id: Option<Uuid>,
        pub written_at: ChronoDateTimeUtc,
        pub session_id: Option<Uuid>,
        pub seal_state: String,
        pub chain_id: Option<Uuid>,
        pub seq: Option<i64>,
        pub prev_hash: Option<Vec<u8>>,
        pub row_hash: Option<Vec<u8>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod audit_log_guard_tests {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::secure::{
        AccessScope, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
    };
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use super::Migrator;
    use super::audit_row::{ActiveModel, Column, Entity, Model};

    const TENANT: Uuid = Uuid::from_u128(0xa0_11);
    const ACTOR: Uuid = Uuid::from_u128(0xa0_22);
    const SUBJECT: Uuid = Uuid::from_u128(0xa0_33);

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom `repo_tests::harness` uses, for the identical reason: a default
    /// `sqlite::memory:` pool hands each checked-out connection its own empty
    /// database, so the migrations applied on one connection would be
    /// invisible on another.
    async fn harness() -> DBProvider<DbError> {
        let opts = ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        };
        let db = connect_db("sqlite::memory:", opts)
            .await
            .expect("connect in-memory sqlite");
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        DBProvider::<DbError>::new(db)
    }

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, hour, 0, 0).unwrap()
    }

    /// A minimal, well-formed `unsealed` row: a subject id is present, every
    /// seam column is absent, as an application's real INSERT would write one.
    fn unsealed_row(audit_id: Uuid) -> ActiveModel {
        ActiveModel {
            audit_id: Set(audit_id),
            tenant_id: Set(TENANT),
            actor_ref: Set(ACTOR),
            action: Set("create".to_owned()),
            subject_kind: Set("product".to_owned()),
            subject_id: Set(Some(SUBJECT)),
            subject_revision: Set(Some(1)),
            error_code: Set(None),
            attempted_key: Set(None),
            reason: Set(Some("initial write".to_owned())),
            correlation_id: Set(None),
            written_at: Set(at(9)),
            session_id: Set(None),
            seal_state: Set("unsealed".to_owned()),
            chain_id: Set(None),
            seq: Set(None),
            prev_hash: Set(None),
            row_hash: Set(None),
        }
    }

    async fn insert(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        model: ActiveModel,
    ) -> Result<Model, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
    }

    /// An `unsealed` row inserts with all four seam columns `NULL` and reads
    /// back with `seal_state = 'unsealed'` — the shape every real write takes
    /// before the platform sealing capability ever runs.
    #[tokio::test]
    async fn an_unsealed_row_inserts_with_every_seam_column_null_and_reads_back() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(1);

        let row = insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert unsealed row");

        assert_eq!(row.seal_state, "unsealed");
        assert_eq!(row.chain_id, None);
        assert_eq!(row.seq, None);
        assert_eq!(row.prev_hash, None);
        assert_eq!(row.row_hash, None);
    }

    /// The seam is reserved, never written by this gear: an `INSERT` claiming
    /// `seal_state = 'sealed'` up front is refused by
    /// `chk_products_audit_log_seal_group`, since a real seal always supplies
    /// `chain_id`, `seq` and `row_hash` together and an application never has
    /// those at INSERT time.
    #[tokio::test]
    async fn an_insert_claiming_sealed_is_refused_by_the_seal_group_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let mut model = unsealed_row(Uuid::from_u128(2));
        model.seal_state = Set("sealed".to_owned());

        let result = insert(&provider, &scope, model).await;

        assert!(
            result.is_err(),
            "a sealed row minted at INSERT time is a half-populated seal that CHECK exists to refuse"
        );
    }

    /// An `unsealed` row that supplies `row_hash` is refused by the same
    /// seal-group `CHECK` from its other arm — no half-populated row, even
    /// one that stayed `unsealed`, is admitted.
    #[tokio::test]
    async fn an_unsealed_row_carrying_a_row_hash_is_refused_by_the_seal_group_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let mut model = unsealed_row(Uuid::from_u128(3));
        model.row_hash = Set(Some(vec![1, 2, 3]));

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// A row carrying neither `subject_id` nor `attempted_key` is refused by
    /// `chk_products_audit_log_subject_ref` — an audit row must never name an
    /// id that identifies nothing, and it must never carry no reference at
    /// all either.
    #[tokio::test]
    async fn a_row_with_neither_subject_id_nor_attempted_key_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let mut model = unsealed_row(Uuid::from_u128(4));
        model.subject_id = Set(None);
        model.attempted_key = Set(None);

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// `DELETE` is refused unconditionally, on any row, sealed or not — the
    /// retention arm is owed to a later slice's `RetentionClock` and this
    /// migration ships no permissive interim.
    #[tokio::test]
    async fn a_delete_of_any_audit_row_is_refused_by_the_trigger() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(5);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to delete");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::delete_many()
            .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(result.is_err());
    }

    /// An `UPDATE` that is not the sealing transition — here, rewriting
    /// `reason` — is refused: the whitelist admits exactly one shape, and a
    /// record column is never it.
    #[tokio::test]
    async fn an_update_that_is_not_the_sealing_transition_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(6);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to update");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::Reason, Expr::value("rewritten"))
            .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(result.is_err());
    }

    /// The one admitted transition succeeds: `unsealed` to `sealed`,
    /// supplying `chain_id`, `seq` and `row_hash` together in the one
    /// statement, `prev_hash` left `NULL` as a segment head.
    #[tokio::test]
    async fn the_admitted_sealing_update_succeeds_leaving_prev_hash_null_as_a_segment_head() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(7);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to seal");
        let chain_id = Uuid::from_u128(70);

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::SealState, Expr::value("sealed"))
            .col_expr(Column::ChainId, Expr::value(chain_id))
            .col_expr(Column::Seq, Expr::value(1_i64))
            .col_expr(Column::RowHash, Expr::value(vec![9_u8, 9, 9]))
            .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;
        assert!(
            result.is_ok(),
            "the sealing transition is the one admitted UPDATE"
        );

        let sealed = Entity::find()
            .secure()
            .scope_with(&scope)
            .and_id(audit_id)
            .expect("resource-scoped find")
            .one(&conn)
            .await
            .expect("read sealed row")
            .expect("the row exists");
        assert_eq!(sealed.seal_state, "sealed");
        assert_eq!(sealed.chain_id, Some(chain_id));
        assert_eq!(sealed.seq, Some(1));
        assert_eq!(sealed.prev_hash, None);
        assert_eq!(sealed.row_hash, Some(vec![9, 9, 9]));
    }

    /// A seal that **supplies** `prev_hash` is admitted too — and this is the
    /// case that makes the seam a chain rather than a pile of segment heads.
    ///
    /// `prev_hash` is one of the four columns the seal writes, not one of the
    /// record columns held unchanged. An earlier draft of the guard listed it
    /// among the unchanged ones, which pinned it at the `NULL` every
    /// `unsealed` row carries: every sealed row would have been a segment head
    /// and no row could ever link to its predecessor. The sibling test above
    /// passes either way — a `NULL` `prev_hash` is legitimate — so only this
    /// case can tell the two guards apart.
    #[tokio::test]
    async fn a_sealing_update_that_supplies_prev_hash_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(0x5ea1);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to seal");
        let chain_id = Uuid::from_u128(0x5ea1_0000);

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::SealState, Expr::value("sealed"))
            .col_expr(Column::ChainId, Expr::value(chain_id))
            .col_expr(Column::Seq, Expr::value(2_i64))
            .col_expr(Column::PrevHash, Expr::value(vec![1_u8, 2, 3]))
            .col_expr(Column::RowHash, Expr::value(vec![4_u8, 5, 6]))
            .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;
        assert!(
            result.is_ok(),
            "the seal supplies prev_hash; a guard that held it unchanged would refuse every non-head row"
        );

        let sealed = Entity::find()
            .secure()
            .scope_with(&scope)
            .and_id(audit_id)
            .expect("resource-scoped find")
            .one(&conn)
            .await
            .expect("read sealed row")
            .expect("the row exists");
        assert_eq!(sealed.prev_hash, Some(vec![1, 2, 3]));
        assert_eq!(sealed.row_hash, Some(vec![4, 5, 6]));
        assert_eq!(sealed.seq, Some(2));
    }

    /// A second sealing `UPDATE` on a row already `sealed` is refused — the
    /// transition is one-way, never re-armed once the row has crossed it.
    #[tokio::test]
    async fn a_second_sealing_update_on_an_already_sealed_row_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(8);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to seal");
        let conn = provider.conn().expect("scoped connection");
        let seal = |chain_id: Uuid| {
            Entity::update_many()
                .col_expr(Column::SealState, Expr::value("sealed"))
                .col_expr(Column::ChainId, Expr::value(chain_id))
                .col_expr(Column::Seq, Expr::value(1_i64))
                .col_expr(Column::RowHash, Expr::value(vec![1_u8]))
                .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
                .secure()
                .scope_with(&scope)
                .exec(&conn)
        };
        seal(Uuid::from_u128(80))
            .await
            .expect("first seal admitted");

        let second = seal(Uuid::from_u128(81)).await;

        assert!(
            second.is_err(),
            "OLD.seal_state is already 'sealed', so the one-way arm no longer admits"
        );
    }

    /// A sealing `UPDATE` that also changes a record column (`reason`) is
    /// refused — every column outside the four seam columns must be
    /// unchanged for the transition to be admitted at all.
    #[tokio::test]
    async fn a_sealing_update_that_also_changes_a_record_column_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let audit_id = Uuid::from_u128(9);
        insert(&provider, &scope, unsealed_row(audit_id))
            .await
            .expect("insert row to seal");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::SealState, Expr::value("sealed"))
            .col_expr(Column::ChainId, Expr::value(Uuid::from_u128(90)))
            .col_expr(Column::Seq, Expr::value(1_i64))
            .col_expr(Column::RowHash, Expr::value(vec![1_u8]))
            .col_expr(Column::Reason, Expr::value("rewritten during sealing"))
            .filter(Condition::all().add(Column::AuditId.eq(audit_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(result.is_err());
    }
}

/// A local, test-only `SeaORM` entity for `products_identity_ref`.
///
/// Not the gear's production entity — resolution and minting land with the
/// repository in the next slice — but the schema this migration creates,
/// scoped through the same `SecureORM` wrappers a real repository would use,
/// so the tests below exercise the migration's own guards rather than a
/// hand-rolled connection.
mod identity_ref_row {
    use sea_orm::entity::prelude::*;
    use toolkit_db_macros::Scopable;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
    #[sea_orm(table_name = "products_identity_ref")]
    #[secure(
        tenant_col = "tenant_id",
        resource_col = "actor_ref",
        no_owner,
        no_type
    )]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub actor_ref: Uuid,
        pub principal_ref: String,
        pub identity_payload: Option<String>,
        pub tombstoned_at: Option<ChronoDateTimeUtc>,
        pub first_seen_at: ChronoDateTimeUtc,
        pub last_seen_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod identity_ref_guard_tests {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::sea_query::{Expr, Value};
    use sea_orm::{ColumnTrait, Condition, EntityTrait};
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::secure::{AccessScope, ScopeError, SecureInsertExt, SecureUpdateExt};
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use super::Migrator;
    use super::identity_ref_row::{ActiveModel, Column, Entity, Model};

    const TENANT_A: Uuid = Uuid::from_u128(0xb0_11);
    const TENANT_B: Uuid = Uuid::from_u128(0xb0_22);
    const PRINCIPAL: &str = "principal-ada";

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom `audit_log_guard_tests::harness` uses, for the identical reason:
    /// a default `sqlite::memory:` pool hands each checked-out connection its
    /// own empty database, so migrations applied on one connection would be
    /// invisible on another.
    async fn harness() -> DBProvider<DbError> {
        let opts = ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        };
        let db = connect_db("sqlite::memory:", opts)
            .await
            .expect("connect in-memory sqlite");
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        DBProvider::<DbError>::new(db)
    }

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, hour, 0, 0).unwrap()
    }

    /// A minimal, well-formed live ref: no tombstone, a non-`NULL` payload,
    /// `last_seen_at` at or after `first_seen_at` — the shape a real mint
    /// writes.
    fn live_ref(tenant_id: Uuid, actor_ref: Uuid, principal_ref: &str) -> ActiveModel {
        ActiveModel {
            tenant_id: Set(tenant_id),
            actor_ref: Set(actor_ref),
            principal_ref: Set(principal_ref.to_owned()),
            identity_payload: Set(Some("{\"name\":\"Ada\"}".to_owned())),
            tombstoned_at: Set(None),
            first_seen_at: Set(at(9)),
            last_seen_at: Set(at(9)),
        }
    }

    async fn insert(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        model: ActiveModel,
    ) -> Result<Model, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
    }

    /// A live ref inserts with a `NULL` `tombstoned_at` and a non-`NULL`
    /// `identity_payload`, and reads back — the shape every first-appearance
    /// mint writes.
    #[tokio::test]
    async fn a_live_ref_inserts_with_null_tombstone_and_non_null_payload_and_reads_back() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let actor_ref = Uuid::from_u128(1);

        let row = insert(&provider, &scope, live_ref(TENANT_A, actor_ref, PRINCIPAL))
            .await
            .expect("insert live ref");

        assert_eq!(row.tombstoned_at, None);
        assert_eq!(row.identity_payload, Some("{\"name\":\"Ada\"}".to_owned()));
        assert_eq!(row.principal_ref, PRINCIPAL);
    }

    /// A second live ref for the same `(tenant_id, principal_ref)` is refused
    /// by `uq_products_identity_ref_active` — L5's one-active-ref-per-principal
    /// rule, physically enforced.
    #[tokio::test]
    async fn a_second_live_ref_for_the_same_principal_is_refused_by_the_active_unique_index() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        insert(
            &provider,
            &scope,
            live_ref(TENANT_A, Uuid::from_u128(2), PRINCIPAL),
        )
        .await
        .expect("insert first live ref");

        let second = insert(
            &provider,
            &scope,
            live_ref(TENANT_A, Uuid::from_u128(3), PRINCIPAL),
        )
        .await;

        assert!(
            second.is_err(),
            "a second live ref for a principal that already has one is exactly what L5 refuses"
        );
    }

    /// After the first ref is tombstoned (`tombstoned_at` set,
    /// `identity_payload` nulled), a fresh ref for the same principal inserts
    /// successfully — a principal acting after erasure mints a new ref, and
    /// the old one stays retired rather than being reused.
    #[tokio::test]
    async fn a_fresh_ref_after_tombstoning_the_first_inserts_successfully() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let retired = Uuid::from_u128(4);
        let first = insert(&provider, &scope, live_ref(TENANT_A, retired, PRINCIPAL))
            .await
            .expect("insert first live ref");

        let conn = provider.conn().expect("scoped connection");
        Entity::update_many()
            .secure()
            .scope_with(&scope)
            .col_expr(Column::TombstonedAt, Expr::value(at(10)))
            .col_expr(Column::IdentityPayload, Expr::value(Value::String(None)))
            .filter(Condition::all().add(Column::ActorRef.eq(first.actor_ref)))
            .exec(&conn)
            .await
            .expect("tombstone the first ref");

        let fresh = insert(
            &provider,
            &scope,
            live_ref(TENANT_A, Uuid::from_u128(5), PRINCIPAL),
        )
        .await;

        assert!(
            fresh.is_ok(),
            "the partial index only guards live rows, so a fresh mint after a tombstone must succeed"
        );
    }

    /// A row carrying both a `tombstoned_at` and a non-`NULL`
    /// `identity_payload` is refused by `chk_products_identity_ref_tombstone`
    /// — a tombstone destroys the payload, it never coexists with it.
    #[tokio::test]
    async fn a_row_with_both_tombstone_and_payload_is_refused_by_the_tombstone_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let mut model = live_ref(TENANT_A, Uuid::from_u128(6), PRINCIPAL);
        model.tombstoned_at = Set(Some(at(10)));

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// A row whose `last_seen_at` precedes its `first_seen_at` is refused by
    /// `chk_products_identity_ref_seen_order`.
    #[tokio::test]
    async fn a_row_with_last_seen_before_first_seen_is_refused_by_the_seen_order_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let mut model = live_ref(TENANT_A, Uuid::from_u128(7), PRINCIPAL);
        model.first_seen_at = Set(at(10));
        model.last_seen_at = Set(at(9));

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// Two refs for the same principal in different tenants both insert — the
    /// active-ref uniqueness is tenant-scoped, not global to the principal.
    #[tokio::test]
    async fn two_refs_for_the_same_principal_in_different_tenants_both_insert() {
        let provider = harness().await;
        let scope_a = AccessScope::for_tenant(TENANT_A);
        let scope_b = AccessScope::for_tenant(TENANT_B);

        let first = insert(
            &provider,
            &scope_a,
            live_ref(TENANT_A, Uuid::from_u128(8), PRINCIPAL),
        )
        .await;
        let second = insert(
            &provider,
            &scope_b,
            live_ref(TENANT_B, Uuid::from_u128(9), PRINCIPAL),
        )
        .await;

        assert!(first.is_ok());
        assert!(second.is_ok());
    }
}

/// A local, test-only `SeaORM` entity for `products_idempotency`.
///
/// Not the gear's production entity — the repository and the claim `INSERT`
/// land in a later slice — but the schema this migration creates, scoped
/// through the same `SecureORM` wrappers a real repository would use, so the
/// tests below exercise the migration's own guards rather than a hand-rolled
/// connection.
mod idempotency_row {
    use sea_orm::entity::prelude::*;
    use toolkit_db_macros::Scopable;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
    #[sea_orm(table_name = "products_idempotency")]
    #[secure(
        tenant_col = "tenant_id",
        resource_col = "client_key",
        no_owner,
        no_type
    )]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub endpoint: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub client_key: String,
        pub state: String,
        pub payload_hash: Vec<u8>,
        pub response_status: Option<i32>,
        pub response_body: Option<String>,
        pub expires_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod idempotency_guard_tests {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait;
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::secure::{AccessScope, ScopeError, SecureInsertExt};
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use super::Migrator;
    use super::idempotency_row::{ActiveModel, Entity, Model};

    const TENANT_A: Uuid = Uuid::from_u128(0xc0_11);
    const TENANT_B: Uuid = Uuid::from_u128(0xc0_22);
    const ENDPOINT: &str = "/bss-products/v1/products/p-1";
    const CLIENT_KEY: &str = "client-key-ada";

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom `audit_log_guard_tests::harness` and `identity_ref_guard_tests::harness`
    /// use, for the identical reason: a default `sqlite::memory:` pool hands
    /// each checked-out connection its own empty database, so migrations
    /// applied on one connection would be invisible on another.
    async fn harness() -> DBProvider<DbError> {
        let opts = ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        };
        let db = connect_db("sqlite::memory:", opts)
            .await
            .expect("connect in-memory sqlite");
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        DBProvider::<DbError>::new(db)
    }

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, hour, 0, 0).unwrap()
    }

    /// A minimal, well-formed `claimed` row: both response columns absent, as
    /// the claim `INSERT` a real door issues would write one before the
    /// guarded mutation has produced anything to store.
    fn claimed_row(tenant_id: Uuid, endpoint: &str, client_key: &str) -> ActiveModel {
        ActiveModel {
            tenant_id: Set(tenant_id),
            endpoint: Set(endpoint.to_owned()),
            client_key: Set(client_key.to_owned()),
            state: Set("claimed".to_owned()),
            payload_hash: Set(vec![1, 2, 3]),
            response_status: Set(None),
            response_body: Set(None),
            expires_at: Set(at(9)),
        }
    }

    /// A minimal, well-formed `answered` row: both response columns present,
    /// as the answer write a real door issues joins the mutation's
    /// transaction and commits with it on success.
    fn answered_row(tenant_id: Uuid, endpoint: &str, client_key: &str) -> ActiveModel {
        ActiveModel {
            tenant_id: Set(tenant_id),
            endpoint: Set(endpoint.to_owned()),
            client_key: Set(client_key.to_owned()),
            state: Set("answered".to_owned()),
            payload_hash: Set(vec![1, 2, 3]),
            response_status: Set(Some(200)),
            response_body: Set(Some("{\"id\":\"p-1\"}".to_owned())),
            expires_at: Set(at(9)),
        }
    }

    async fn insert(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        model: ActiveModel,
    ) -> Result<Model, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
    }

    /// A `claimed` row inserts with both response columns `NULL` and reads
    /// back — the shape the claim `INSERT` itself writes, before the guarded
    /// mutation has produced anything to answer with.
    #[tokio::test]
    async fn a_claimed_row_inserts_with_both_response_columns_null_and_reads_back() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);

        let row = insert(
            &provider,
            &scope,
            claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY),
        )
        .await
        .expect("insert claimed row");

        assert_eq!(row.state, "claimed");
        assert_eq!(row.response_status, None);
        assert_eq!(row.response_body, None);
    }

    /// A `claimed` row carrying a `response_status` is refused by
    /// `chk_products_idempotency_response_group` — `claimed` admits no
    /// response column at all, half-answered or not.
    #[tokio::test]
    async fn a_claimed_row_carrying_a_response_status_is_refused_by_the_response_group_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let mut model = claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY);
        model.response_status = Set(Some(200));

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// An `answered` row with both response columns present inserts — the
    /// shape the answer write commits on a success, joined to the guarded
    /// mutation's own transaction.
    #[tokio::test]
    async fn an_answered_row_with_both_response_columns_present_inserts() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);

        let row = insert(
            &provider,
            &scope,
            answered_row(TENANT_A, ENDPOINT, CLIENT_KEY),
        )
        .await
        .expect("insert answered row");

        assert_eq!(row.state, "answered");
        assert_eq!(row.response_status, Some(200));
        assert_eq!(row.response_body, Some("{\"id\":\"p-1\"}".to_owned()));
    }

    /// An `answered` row missing `response_body` is refused by
    /// `chk_products_idempotency_response_group` — `answered` requires both
    /// response columns together, never one alone.
    #[tokio::test]
    async fn an_answered_row_missing_response_body_is_refused_by_the_response_group_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let mut model = answered_row(TENANT_A, ENDPOINT, CLIENT_KEY);
        model.response_body = Set(None);

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// A `state` outside the `claimed`/`answered` roster is refused by
    /// `chk_products_idempotency_state`.
    #[tokio::test]
    async fn a_state_outside_the_roster_is_refused_by_the_state_check() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        let mut model = claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY);
        model.state = Set("in_flight".to_owned());

        let result = insert(&provider, &scope, model).await;

        assert!(result.is_err());
    }

    /// Two rows differing only in `tenant_id` both insert — the key is
    /// tenant-scoped, not global to `(endpoint, client_key)`.
    #[tokio::test]
    async fn two_rows_differing_only_in_tenant_id_both_insert() {
        let provider = harness().await;
        let scope_a = AccessScope::for_tenant(TENANT_A);
        let scope_b = AccessScope::for_tenant(TENANT_B);

        let first = insert(
            &provider,
            &scope_a,
            claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY),
        )
        .await;
        let second = insert(
            &provider,
            &scope_b,
            claimed_row(TENANT_B, ENDPOINT, CLIENT_KEY),
        )
        .await;

        assert!(first.is_ok());
        assert!(second.is_ok());
    }

    /// A second row on the same `(tenant_id, endpoint, client_key)` is
    /// refused by the primary key.
    ///
    /// This is the load-bearing case: it is what makes the claim `INSERT` a
    /// gate rather than a hint. A read-then-write check would admit both of
    /// two concurrent requests carrying the same client key, which is the one
    /// situation an idempotency key exists to prevent — the composite primary
    /// key is what makes exactly one of two racing inserts win.
    #[tokio::test]
    async fn a_second_row_on_the_same_key_is_refused_by_the_primary_key() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT_A);
        insert(
            &provider,
            &scope,
            claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY),
        )
        .await
        .expect("insert first row on the key");

        let second = insert(
            &provider,
            &scope,
            claimed_row(TENANT_A, ENDPOINT, CLIENT_KEY),
        )
        .await;

        assert!(
            second.is_err(),
            "the composite primary key is the at-most-once gate itself"
        );
    }
}

/// A local, test-only `SeaORM` entity for `products_product`.
///
/// Not the gear's production entity — that lives at
/// `infra::storage::entity::product` and is exercised by `repo_tests` — but
/// the identical schema this migration creates, scoped through the same
/// `SecureORM` wrappers a real repository would use, so the tests below
/// exercise the migration's own append-only guard rather than a hand-rolled
/// connection.
mod product_row {
    use sea_orm::entity::prelude::*;
    use toolkit_db_macros::Scopable;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
    #[sea_orm(table_name = "products_product")]
    #[secure(
        tenant_col = "tenant_id",
        resource_col = "product_id",
        no_owner,
        no_type
    )]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub product_id: Uuid,
        pub tenant_id: Uuid,
        pub brand_id: Uuid,
        pub name: String,
        pub name_normalized: String,
        pub product_code: Option<String>,
        pub lifecycle_state: String,
        pub internal_revision: i64,
        pub published_version: i64,
        pub region_scope: String,
        pub brand_scope: String,
        pub created_by: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A local, test-only `SeaORM` entity for `products_sku`.
///
/// Not the gear's production entity — that lives at
/// `infra::storage::entity::sku` and is exercised by `repo_tests` — but the
/// identical schema this migration creates, scoped through the same
/// `SecureORM` wrappers a real repository would use.
mod sku_row {
    use sea_orm::entity::prelude::*;
    use toolkit_db_macros::Scopable;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
    #[sea_orm(table_name = "products_sku")]
    #[secure(tenant_col = "tenant_id", resource_col = "sku_id", no_owner, no_type)]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub sku_id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub sku_code: String,
        pub lifecycle_state: String,
        pub internal_revision: i64,
        pub published_version: i64,
        pub region_scope: String,
        pub brand_scope: String,
        pub created_by: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `products_product`'s append-only head-row guard
/// (`cpt-cf-bss-products-dod-append-only-guard`), exercised against the
/// executed `SQLite` mirror.
///
/// Every refusal probe below is paired with a positive control: a probe that
/// only shows a write refused could pass on a guard that refuses every
/// `UPDATE` unconditionally, and a probe that only shows a write admitted
/// could pass on a guard that admits every `UPDATE` unconditionally. Only the
/// pair together tells a correct whitelist apart from either extreme.
mod product_guard_tests {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::secure::{
        AccessScope, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
    };
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use super::Migrator;
    use super::product_row::{ActiveModel, Column, Entity, Model};

    const TENANT: Uuid = Uuid::from_u128(0xd0_11);
    const BRAND: Uuid = Uuid::from_u128(0xd0_22);
    const OTHER_BRAND: Uuid = Uuid::from_u128(0xd0_33);

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom every other guard-test module in this file uses, for the
    /// identical reason: a default `sqlite::memory:` pool hands each
    /// checked-out connection its own empty database.
    async fn harness() -> DBProvider<DbError> {
        let opts = ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        };
        let db = connect_db("sqlite::memory:", opts)
            .await
            .expect("connect in-memory sqlite");
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        DBProvider::<DbError>::new(db)
    }

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, hour, 0, 0).unwrap()
    }

    /// A minimal, well-formed `draft` head, unpublished (`published_version =
    /// 0`), as a real `create` door writes one.
    fn draft_row(product_id: Uuid, name: &str) -> ActiveModel {
        ActiveModel {
            product_id: Set(product_id),
            tenant_id: Set(TENANT),
            brand_id: Set(BRAND),
            name: Set(name.to_owned()),
            name_normalized: Set(name.to_owned()),
            product_code: Set(Some(format!("code-{name}"))),
            lifecycle_state: Set("draft".to_owned()),
            internal_revision: Set(1),
            published_version: Set(0),
            region_scope: Set(String::new()),
            brand_scope: Set(String::new()),
            created_by: Set("actor-ada".to_owned()),
            created_at: Set(at(9)),
            updated_at: Set(at(9)),
        }
    }

    async fn insert(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        model: ActiveModel,
    ) -> Result<Model, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
    }

    /// Issues an `UPDATE` against exactly the `(column, value)` pairs given,
    /// filtered to one row — the precise shape needed to probe one guard
    /// clause without incidentally tripping another.
    async fn update(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        product_id: Uuid,
        cols: Vec<(Column, sea_orm::Value)>,
    ) -> Result<sea_orm::UpdateResult, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        let mut q = Entity::update_many();
        for (col, val) in cols {
            q = q.col_expr(col, Expr::value(val));
        }
        q.filter(Condition::all().add(Column::ProductId.eq(product_id)))
            .secure()
            .scope_with(scope)
            .exec(&conn)
            .await
    }

    /// Asserts an admitted `UPDATE` actually matched a row.
    ///
    /// `SeaORM` does not store a `Uuid` in `SQLite` as hyphenated text, so a
    /// filter built the wrong way can silently match nothing and still
    /// report `Ok`. Every positive-control assertion below checks
    /// `rows_affected` rather than `is_ok()` alone, so a miss cannot pass as
    /// a pass.
    fn assert_applied(result: &Result<sea_orm::UpdateResult, ScopeError>, why: &str) {
        match result {
            Ok(outcome) => assert!(outcome.rows_affected > 0, "{why}: matched no row"),
            Err(err) => panic!("{why}: expected the update to be admitted, got {err}"),
        }
    }

    /// Class 1/2 (bucket-i): refused after first publish, admitted before it.
    ///
    /// This is the refusal half — `published_version = 1` (already
    /// published) — proving the guard reaches back further than "currently
    /// draft."
    #[tokio::test]
    async fn bucket_i_change_is_refused_once_published_version_is_above_zero() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(1);
        insert(&provider, &scope, draft_row(id, "alpha"))
            .await
            .expect("insert draft");
        update(
            &provider,
            &scope,
            id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await
        .expect("publish the head");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::BrandId, OTHER_BRAND.into()),
                (Column::InternalRevision, 3i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "bucket-i is refused once published_version leaves zero, per the `DoD`'s own words \"never after first publish\""
        );
    }

    /// Class 1/2 (bucket-i): the positive control for the probe above — the
    /// identical `brand_id` change succeeds while `published_version = 0`
    /// and the head is `draft` (non-terminal). Without this, the refusal
    /// above could pass on a guard that refuses every `UPDATE`.
    #[tokio::test]
    async fn bucket_i_change_is_admitted_while_unpublished_and_non_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(2);
        insert(&provider, &scope, draft_row(id, "bravo"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::BrandId, OTHER_BRAND.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a bucket-i change on an unpublished, non-terminal draft is exactly what the guard admits",
        );
        let conn = provider.conn().expect("scoped connection");
        let row = Entity::find()
            .secure()
            .scope_with(&scope)
            .and_id(id)
            .expect("resource-scoped find")
            .one(&conn)
            .await
            .expect("read row")
            .expect("row exists");
        assert_eq!(row.brand_id, OTHER_BRAND);
    }

    /// Class 3 (bucket-i, terminal): refused while the head is terminal, even
    /// though `published_version` is still zero — a never-published draft
    /// that was discarded does not reopen bucket-i.
    #[tokio::test]
    async fn bucket_i_change_is_refused_while_the_head_is_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(3);
        let mut row = draft_row(id, "charlie");
        row.lifecycle_state = Set("discarded".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::ProductCode, Some("new-code".to_owned()).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "published_version = 0 alone is not enough; the head must also be non-terminal"
        );
    }

    /// Class 4 (bucket-iii): refused while the head is terminal.
    #[tokio::test]
    async fn bucket_iii_change_is_refused_while_the_head_is_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(4);
        let mut row = draft_row(id, "delta");
        row.lifecycle_state = Set("retired".to_owned());
        row.published_version = Set(1);
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "bucket-iii is refused once the head has reached a terminal state"
        );
    }

    /// Class 4 (bucket-iii): the positive control — the identical
    /// `region_scope` change succeeds on a non-terminal head, including one
    /// already published, since bucket-iii carries no `published_version`
    /// gate.
    #[tokio::test]
    async fn bucket_iii_change_is_admitted_while_the_head_is_non_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(5);
        let mut row = draft_row(id, "echo");
        row.lifecycle_state = Set("published".to_owned());
        row.published_version = Set(1);
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a published, non-terminal head may still be rescoped under governance",
        );
    }

    /// Class 5 (`lifecycle_state`): a transition off the edge list —
    /// `draft` straight to `retired`, skipping `published`/`deprecated` — is
    /// refused.
    #[tokio::test]
    async fn a_lifecycle_transition_off_the_edge_list_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(6);
        insert(&provider, &scope, draft_row(id, "foxtrot"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::LifecycleState, "retired".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "draft -> retired is not one of the five admitted edges"
        );
    }

    /// Class 5 (`lifecycle_state`): the positive control — `draft ->
    /// published`, one of the five admitted edges, succeeds.
    #[tokio::test]
    async fn a_lifecycle_transition_along_the_edge_list_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(7);
        insert(&provider, &scope, draft_row(id, "golf"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::LifecycleState, "published".to_owned().into()),
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(&result, "draft -> published is an admitted edge");
    }

    /// Class 6 (`published_version`): refused for a `+2` jump.
    #[tokio::test]
    async fn published_version_moving_by_two_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(8);
        insert(&provider, &scope, draft_row(id, "hotel"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::PublishedVersion, 2i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(result.is_err(), "published_version only ever moves by +1");
    }

    /// Class 6 (`published_version`): refused for a decrement.
    #[tokio::test]
    async fn published_version_decrementing_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(9);
        let mut row = draft_row(id, "india");
        row.published_version = Set(2);
        row.lifecycle_state = Set("published".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(result.is_err(), "published_version never moves backward");
    }

    /// Class 6 (`published_version`): the positive control — a bare `+1`
    /// with no other change succeeds.
    #[tokio::test]
    async fn published_version_moving_by_exactly_one_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(10);
        insert(&provider, &scope, draft_row(id, "juliet"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await;

        assert_applied(&result, "+1 is the one admitted published_version move");
    }

    /// Class 7 (`internal_revision`): an otherwise-admitted update (a bucket-
    /// iii rename) that leaves `internal_revision` unmoved is refused — the
    /// clause has no carve-out.
    #[tokio::test]
    async fn an_update_that_does_not_bump_internal_revision_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(11);
        insert(&provider, &scope, draft_row(id, "kilo"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![(Column::RegionScope, "eu".to_owned().into())],
        )
        .await;

        assert!(
            result.is_err(),
            "internal_revision must move by exactly one on every admitted update, without exception"
        );
    }

    /// Class 7 (`internal_revision`): a `+2` jump is refused too — "exactly
    /// one," not "at least one."
    #[tokio::test]
    async fn an_update_that_bumps_internal_revision_by_two_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(12);
        insert(&provider, &scope, draft_row(id, "lima"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 3i64.into()),
            ],
        )
        .await;

        assert!(result.is_err(), "a +2 jump is not \"exactly one\"");
    }

    /// Class 7 (`internal_revision`): the positive control — the identical
    /// rename with a `+1` bump succeeds.
    #[tokio::test]
    async fn an_update_that_bumps_internal_revision_by_exactly_one_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(13);
        insert(&provider, &scope, draft_row(id, "mike"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(&result, "a +1 bump is the one admitted shape");
    }

    /// Class 8: `tenant_id` is refused in any update, even paired with an
    /// otherwise-admitted `internal_revision` bump.
    #[tokio::test]
    async fn tenant_id_is_refused_in_any_update() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(14);
        insert(&provider, &scope, draft_row(id, "november"))
            .await
            .expect("insert draft");
        let other_tenant = Uuid::from_u128(0xd0_99);

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::TenantId, other_tenant.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "tenant_id is admitted in no update at all (P-D-34)"
        );
    }

    /// Class 8: the primary key, `product_id`, is refused in any update.
    #[tokio::test]
    async fn product_id_is_refused_in_any_update() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(15);
        insert(&provider, &scope, draft_row(id, "oscar"))
            .await
            .expect("insert draft");
        let other_id = Uuid::from_u128(0xd0_98);

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::ProductId, other_id.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "the primary key is admitted in no update at all (P-D-34)"
        );
    }

    /// Class 8: `created_by` is refused in any update.
    #[tokio::test]
    async fn created_by_is_refused_in_any_update() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(16);
        insert(&provider, &scope, draft_row(id, "papa"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::CreatedBy, "actor-eve".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "created_by is admitted in no update at all (P-D-34)"
        );
    }

    /// Class 8: `created_at` is refused in any update — the `DoD` does not
    /// name it, but it sits with `tenant_id`/the primary key/`created_by`
    /// for the identical reason: none of the four is ever supplied by an
    /// `UPDATE`, only by the `INSERT`.
    #[tokio::test]
    async fn created_at_is_refused_in_any_update() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(17);
        insert(&provider, &scope, draft_row(id, "quebec"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::CreatedAt, at(10).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(result.is_err(), "created_at never moves after INSERT");
    }

    /// `DELETE` is refused unconditionally, matching the C5 append-only
    /// posture the head-row guard shares with `products_audit_log`.
    #[tokio::test]
    async fn a_delete_of_a_head_row_is_refused_by_the_trigger() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(18);
        insert(&provider, &scope, draft_row(id, "romeo"))
            .await
            .expect("insert draft");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::delete_many()
            .filter(Condition::all().add(Column::ProductId.eq(id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(
            result.is_err(),
            "a head row is retired through lifecycle_state, never removed"
        );
    }

    /// `updated_at` is admitted unconditionally, paired here with the one
    /// other required clause (`internal_revision +1`) since a bare
    /// `updated_at` touch with nothing else is exactly what a save door
    /// issues when no content column changed.
    #[tokio::test]
    async fn updated_at_alone_with_the_revision_bump_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(19);
        insert(&provider, &scope, draft_row(id, "sierra"))
            .await
            .expect("insert draft");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::UpdatedAt, at(11).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(&result, "updated_at is admitted unconditionally");
    }
}

/// `products_sku`'s append-only head-row guard, exercised the identical way
/// as `products_product`'s. The whitelist mirrors the sibling table clause
/// for clause; this module re-proves the classes shaped differently on this
/// table (`sku_code`/`product_id` as bucket-i, no `name` in bucket-iii) and
/// re-runs the shared classes (`lifecycle_state`, `published_version`,
/// `internal_revision`, the immutable set, `DELETE`) to prove both engines'
/// mirrors — Postgres's function and `SQLite`'s per-clause triggers — carry
/// the same whitelist rather than diverging where the schema differs.
mod sku_guard_tests {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::secure::{
        AccessScope, ScopeError, SecureDeleteExt, SecureInsertExt, SecureUpdateExt,
    };
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use super::Migrator;
    use super::product_row::ActiveModel as ProductActiveModel;
    use super::product_row::Entity as ProductEntity;
    use super::sku_row::{ActiveModel, Column, Entity, Model};

    const TENANT: Uuid = Uuid::from_u128(0xe0_11);
    const BRAND: Uuid = Uuid::from_u128(0xe0_22);

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom every other guard-test module in this file uses.
    async fn harness() -> DBProvider<DbError> {
        let opts = ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        };
        let db = connect_db("sqlite::memory:", opts)
            .await
            .expect("connect in-memory sqlite");
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        DBProvider::<DbError>::new(db)
    }

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, hour, 0, 0).unwrap()
    }

    /// A minimal, well-formed parent Product, published — a SKU's foreign key
    /// must resolve to an existing row.
    ///
    /// The name is derived from the id rather than fixed: `uq_products_product_name`
    /// is unique over `(tenant_id, brand_id, name_normalized)` among
    /// non-discarded rows, so a constant name let only one parent exist per
    /// tenant and any test seeding a second one failed on the index rather
    /// than on what it was testing.
    fn parent_row(product_id: Uuid) -> ProductActiveModel {
        let name = format!("parent-{product_id}");
        ProductActiveModel {
            product_id: Set(product_id),
            tenant_id: Set(TENANT),
            brand_id: Set(BRAND),
            name: Set(name.clone()),
            name_normalized: Set(name),
            product_code: Set(None),
            lifecycle_state: Set("published".to_owned()),
            internal_revision: Set(1),
            published_version: Set(1),
            region_scope: Set(String::new()),
            brand_scope: Set(String::new()),
            created_by: Set("actor-ada".to_owned()),
            created_at: Set(at(9)),
            updated_at: Set(at(9)),
        }
    }

    async fn insert_parent(provider: &DBProvider<DbError>, scope: &AccessScope, product_id: Uuid) {
        let conn = provider.conn().expect("scoped connection");
        let model = parent_row(product_id);
        ProductEntity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
            .expect("insert parent product");
    }

    /// A minimal, well-formed `draft` SKU, unpublished, parented to an
    /// already-inserted Product.
    fn draft_row(sku_id: Uuid, product_id: Uuid, code: &str) -> ActiveModel {
        ActiveModel {
            sku_id: Set(sku_id),
            tenant_id: Set(TENANT),
            product_id: Set(product_id),
            sku_code: Set(code.to_owned()),
            lifecycle_state: Set("draft".to_owned()),
            internal_revision: Set(1),
            published_version: Set(0),
            region_scope: Set(String::new()),
            brand_scope: Set(String::new()),
            created_by: Set("actor-ada".to_owned()),
            created_at: Set(at(9)),
            updated_at: Set(at(9)),
        }
    }

    async fn insert(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        model: ActiveModel,
    ) -> Result<Model, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
    }

    async fn update(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        sku_id: Uuid,
        cols: Vec<(Column, sea_orm::Value)>,
    ) -> Result<sea_orm::UpdateResult, ScopeError> {
        let conn = provider.conn().expect("scoped connection");
        let mut q = Entity::update_many();
        for (col, val) in cols {
            q = q.col_expr(col, Expr::value(val));
        }
        q.filter(Condition::all().add(Column::SkuId.eq(sku_id)))
            .secure()
            .scope_with(scope)
            .exec(&conn)
            .await
    }

    /// Asserts an admitted `UPDATE` actually matched a row — see
    /// `product_guard_tests::assert_applied` for why this checks
    /// `rows_affected` rather than `is_ok()` alone.
    fn assert_applied(result: &Result<sea_orm::UpdateResult, ScopeError>, why: &str) {
        match result {
            Ok(outcome) => assert!(outcome.rows_affected > 0, "{why}: matched no row"),
            Err(err) => panic!("{why}: expected the update to be admitted, got {err}"),
        }
    }

    /// Class 1/2 (bucket-i, `sku_code`/`product_id`): refused once published.
    #[tokio::test]
    async fn bucket_i_change_is_refused_once_published_version_is_above_zero() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(1);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(101);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-1"))
            .await
            .expect("insert draft sku");
        update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await
        .expect("publish the sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::SkuCode, "sku-1-renamed".to_owned().into()),
                (Column::InternalRevision, 3i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "sku_code is bucket-i and is refused once published_version leaves zero"
        );
    }

    /// Class 1/2 (bucket-i): the positive control — the same `sku_code`
    /// change succeeds while unpublished and non-terminal.
    #[tokio::test]
    async fn bucket_i_change_is_admitted_while_unpublished_and_non_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(2);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(102);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-2"))
            .await
            .expect("insert draft sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::SkuCode, "sku-2-renamed".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a bucket-i change on an unpublished, non-terminal draft SKU is admitted",
        );
    }

    /// Class 3 (bucket-i, terminal): `product_id` (the parent link) is
    /// refused while the SKU is terminal, even at `published_version = 0`.
    #[tokio::test]
    async fn bucket_i_change_is_refused_while_the_head_is_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(3);
        let other_product_id = Uuid::from_u128(4);
        insert_parent(&provider, &scope, product_id).await;
        insert_parent(&provider, &scope, other_product_id).await;
        let sku_id = Uuid::from_u128(103);
        let mut row = draft_row(sku_id, product_id, "sku-3");
        row.lifecycle_state = Set("discarded".to_owned());
        insert(&provider, &scope, row).await.expect("insert sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::ProductId, other_product_id.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "re-parenting a discarded (terminal) SKU is refused regardless of published_version"
        );
    }

    /// Class 4 (bucket-iii, `region_scope`/`brand_scope`): refused while
    /// terminal, admitted while non-terminal.
    #[tokio::test]
    async fn bucket_iii_change_is_refused_while_terminal_and_admitted_while_non_terminal() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(5);
        insert_parent(&provider, &scope, product_id).await;

        let terminal_id = Uuid::from_u128(104);
        let mut terminal_row = draft_row(terminal_id, product_id, "sku-4");
        terminal_row.lifecycle_state = Set("retired".to_owned());
        terminal_row.published_version = Set(1);
        insert(&provider, &scope, terminal_row)
            .await
            .expect("insert terminal sku");
        let refused = update(
            &provider,
            &scope,
            terminal_id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(refused.is_err(), "bucket-iii is refused once terminal");

        let live_id = Uuid::from_u128(105);
        insert(&provider, &scope, draft_row(live_id, product_id, "sku-5"))
            .await
            .expect("insert draft sku");
        let admitted = update(
            &provider,
            &scope,
            live_id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert_applied(
            &admitted,
            "the positive control: the identical change on a non-terminal (draft) SKU is admitted",
        );
    }

    /// Class 5 (`lifecycle_state`): the shared edge list, refused off it and
    /// admitted along it.
    #[tokio::test]
    async fn lifecycle_transition_is_refused_off_the_edge_list_and_admitted_along_it() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(6);
        insert_parent(&provider, &scope, product_id).await;

        let bad_id = Uuid::from_u128(106);
        insert(&provider, &scope, draft_row(bad_id, product_id, "sku-6"))
            .await
            .expect("insert draft sku");
        let refused = update(
            &provider,
            &scope,
            bad_id,
            vec![
                (Column::LifecycleState, "retired".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(refused.is_err(), "draft -> retired is not an admitted edge");

        let good_id = Uuid::from_u128(107);
        insert(&provider, &scope, draft_row(good_id, product_id, "sku-7"))
            .await
            .expect("insert draft sku");
        let admitted = update(
            &provider,
            &scope,
            good_id,
            vec![
                (Column::LifecycleState, "discarded".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert_applied(&admitted, "draft -> discarded is an admitted edge");
    }

    /// Class 6 (`published_version`): refused for a `+2` jump and a
    /// decrement.
    #[tokio::test]
    async fn published_version_is_refused_for_anything_but_plus_one() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(7);
        insert_parent(&provider, &scope, product_id).await;

        let jump_id = Uuid::from_u128(108);
        insert(&provider, &scope, draft_row(jump_id, product_id, "sku-8"))
            .await
            .expect("insert draft sku");
        let jumped = update(
            &provider,
            &scope,
            jump_id,
            vec![
                (Column::PublishedVersion, 2i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(jumped.is_err(), "a +2 jump is refused");

        let decrement_id = Uuid::from_u128(109);
        let mut row = draft_row(decrement_id, product_id, "sku-9");
        row.published_version = Set(2);
        row.lifecycle_state = Set("published".to_owned());
        insert(&provider, &scope, row).await.expect("insert sku");
        let decremented = update(
            &provider,
            &scope,
            decrement_id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(decremented.is_err(), "a decrement is refused");
    }

    /// Class 6 (`published_version`): the positive control — `+1` succeeds.
    #[tokio::test]
    async fn published_version_moving_by_exactly_one_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(8);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(110);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-10"))
            .await
            .expect("insert draft sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await;

        assert_applied(&result, "+1 is the one admitted published_version move");
    }

    /// Class 7 (`internal_revision`): refused when an otherwise-admitted
    /// update leaves it unmoved, admitted when it moves by exactly one.
    #[tokio::test]
    async fn internal_revision_is_refused_unless_it_moves_by_exactly_one() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(9);
        insert_parent(&provider, &scope, product_id).await;

        let unmoved_id = Uuid::from_u128(111);
        insert(
            &provider,
            &scope,
            draft_row(unmoved_id, product_id, "sku-11"),
        )
        .await
        .expect("insert draft sku");
        let unmoved = update(
            &provider,
            &scope,
            unmoved_id,
            vec![(Column::RegionScope, "eu".to_owned().into())],
        )
        .await;
        assert!(unmoved.is_err(), "no revision bump at all is refused");

        let moved_id = Uuid::from_u128(112);
        insert(&provider, &scope, draft_row(moved_id, product_id, "sku-12"))
            .await
            .expect("insert draft sku");
        let moved = update(
            &provider,
            &scope,
            moved_id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert_applied(&moved, "the positive control: a +1 bump is admitted");
    }

    /// Class 8: `tenant_id`, the primary key (`sku_id`) and `created_by` are
    /// each refused in any update; so is `created_at`.
    #[tokio::test]
    async fn the_immutable_columns_are_each_refused_in_any_update() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(10);
        insert_parent(&provider, &scope, product_id).await;

        let tenant_probe = Uuid::from_u128(201);
        insert(
            &provider,
            &scope,
            draft_row(tenant_probe, product_id, "sku-tenant-probe"),
        )
        .await
        .expect("insert sku");
        let tenant_result = update(
            &provider,
            &scope,
            tenant_probe,
            vec![
                (Column::TenantId, Uuid::from_u128(0xe0_99).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(
            tenant_result.is_err(),
            "tenant_id is admitted in no update at all (P-D-34)"
        );

        let primary_key_probe = Uuid::from_u128(202);
        insert(
            &provider,
            &scope,
            draft_row(primary_key_probe, product_id, "sku-primary-key-probe"),
        )
        .await
        .expect("insert sku");
        let pk_result = update(
            &provider,
            &scope,
            primary_key_probe,
            vec![
                (Column::SkuId, Uuid::from_u128(0xe0_98).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(
            pk_result.is_err(),
            "the primary key is admitted in no update at all (P-D-34)"
        );

        let creator_probe = Uuid::from_u128(203);
        insert(
            &provider,
            &scope,
            draft_row(creator_probe, product_id, "sku-creator-probe"),
        )
        .await
        .expect("insert sku");
        let created_by_result = update(
            &provider,
            &scope,
            creator_probe,
            vec![
                (Column::CreatedBy, "actor-eve".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(
            created_by_result.is_err(),
            "created_by is admitted in no update at all (P-D-34)"
        );

        let mint_timestamp_probe = Uuid::from_u128(204);
        insert(
            &provider,
            &scope,
            draft_row(mint_timestamp_probe, product_id, "sku-mint-timestamp-probe"),
        )
        .await
        .expect("insert sku");
        let created_at_result = update(
            &provider,
            &scope,
            mint_timestamp_probe,
            vec![
                (Column::CreatedAt, at(10).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;
        assert!(
            created_at_result.is_err(),
            "created_at never moves after INSERT"
        );
    }

    /// `DELETE` is refused unconditionally on `products_sku` too — the same
    /// C5 append-only posture as `products_product`.
    #[tokio::test]
    async fn a_delete_of_a_sku_head_row_is_refused_by_the_trigger() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(11);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(113);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-13"))
            .await
            .expect("insert draft sku");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::delete_many()
            .filter(Condition::all().add(Column::SkuId.eq(sku_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(
            result.is_err(),
            "a SKU head row is retired through lifecycle_state, never removed"
        );
    }
}
