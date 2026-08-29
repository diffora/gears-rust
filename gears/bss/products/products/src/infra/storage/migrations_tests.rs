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
