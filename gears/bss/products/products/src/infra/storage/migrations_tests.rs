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
//! otherwise pass on a guard that refuses everything. Since Phase 6 that
//! whitelist includes the `published_version` clause's **existence** half —
//! a bump is admitted only where the matching `products_entity_version` row
//! already exists — asserted for `products_product` and `products_sku`
//! separately, each trigger carrying its own `entity_kind` literal.
//! Since Phase 6 it also includes that clause's **terminal** half — a bump is
//! refused outright on a `retired` or `discarded` head — probed from both
//! sides on both tables, because the clause is gated on the counter moving
//! and must not become a blanket ban on writing a terminal row.
//!
//! Last: `products_entity_version` itself, through the gear's own
//! `entity::entity_version` (the first of these modules that needs no
//! test-only mirror). What it exercises is the migration: the column set, the
//! unique key, the `entity_kind` roster `CHECK`, the two lower-bound `CHECK`s,
//! and the frozen-row guard's unconditional refusal of `UPDATE` and `DELETE`.
//!
//! And last of all, `lifecycle_roster_tests`: the admitted-edge roster is
//! written out in five places — the application's
//! `domain::transition::ADMITTED_EDGES` and four SQL clauses, two per engine
//! per table — and nothing else holds them equal. That module pins all five
//! against one another, reading the `SQLite` halves back out of
//! `sqlite_master` and the Postgres halves out of the migration sources.
//!
//! After it, `bucket_agreement_tests`: §5's agreement test between the
//! `BucketRegistry` (`domain::bucket`) and §4.2's trigger column classes. The
//! registry is **advisory for the physical layer** (P-D-32), so nothing but
//! this module holds the two statements of the bucket rule equal. Three
//! assertions: the same columns in the same classes per entity, with iii and
//! iv combined because the whitelist admits them together; bucket-ii empty on
//! both sides, which is what §4.2's interim row-image predicate has to say
//! about today's columns and is re-pointed when 07 supplies a tighter one;
//! and P-D-50's third, that no published-state column is named by *neither*
//! artifact — the case the first two are blind to by construction, and
//! exactly the column the door's fail-closed miss would refuse at runtime.
//! Both sides are read rather than restated: the executed triggers and the
//! executed column list out of the engine, the Postgres clauses out of the
//! migration source.
//!
//! **Only the `SQLite` mirror is executed.** The whole suite runs in-memory on
//! `SQLite`, so no test in this file executes a Postgres statement; the
//! `PL/pgSQL` halves of every guard asserted here were compared to their
//! `SQLite` counterparts clause for clause **by reading** — except the
//! lifecycle edge list and the bucket column classes, which
//! `lifecycle_roster_tests` and `bucket_agreement_tests` now compare
//! mechanically, from the Postgres source text since there is no executed
//! Postgres artifact to read — which makes those halves **source agreement,
//! not engine behaviour**.
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

/// A trigger created by one migration may reference a table a **later**
/// migration creates.
///
/// `m20260829_000002_create_products_product` and
/// `m20260829_000003_create_products_sku` install a `published_version` guard
/// whose existence clause reads `products_entity_version`, which
/// `m20260829_000007_create_products_entity_version` creates — and the runner
/// applies migrations in **name** order, so `000007` runs after both. That is
/// admitted because a trigger body is late-bound on both engines
/// (`PL/pgSQL` function bodies and `SQLite` trigger bodies resolve table names
/// at execution, not at creation), but "admitted in theory" is a claim, so
/// this test measures it: it asserts the ordering is genuinely the awkward
/// one, and then boots the whole chain.
///
/// A boot is the assertion. Were `SQLite` to resolve the referenced table at
/// `CREATE TRIGGER` time, statement 5 of `000002` would fail and
/// `run_migrations_for_testing` would return an error here.
/// `product_guard_tests::a_published_version_bump_without_its_version_row_is_refused`
/// carries the other half — that the clause does resolve, and does refuse, at
/// `UPDATE` time.
#[tokio::test]
async fn a_trigger_may_reference_a_table_a_later_migration_creates() {
    use toolkit_db::{ConnectOpts, connect_db};

    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let head = names
        .iter()
        .position(|n| n == "m20260829_000002_create_products_product")
        .expect("the product migration is in the roster");
    let version = names
        .iter()
        .position(|n| n == "m20260829_000007_create_products_entity_version")
        .expect("the entity-version migration is in the roster");
    assert!(
        head < version,
        "this test is only meaningful while the referencing migration runs first"
    );

    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db("sqlite::memory:", opts)
        .await
        .expect("connect in-memory sqlite");

    let booted =
        toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations()).await;

    assert!(
        booted.is_ok(),
        "the chain must boot with a trigger referencing a table created three migrations later: {booted:?}"
    );
}

/// Seeds `products_entity_version` rows, so the head-row guard's existence
/// half has something to find.
///
/// Unlike the five entities above this is **not** a test-only mirror: it
/// writes through the gear's own `entity::entity_version`, which Phase 6
/// lands alongside the migration. A frozen row seeded here is minimal but
/// well-formed — the `CHECK`s on `entity_kind`, `published_version` and
/// `digest_version` all bite on it, so a mis-seeded row fails loudly here
/// rather than quietly weakening the probe it feeds.
mod frozen_version {
    use chrono::{TimeZone, Utc};
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait;
    use toolkit_db::secure::{AccessScope, SecureInsertExt};
    use toolkit_db::{DBProvider, DbError};
    use uuid::Uuid;

    use crate::infra::storage::entity::entity_version::{ActiveModel, Entity};

    const ACTOR: Uuid = Uuid::from_u128(0xac_70_12);

    /// Freezes `version` of `(kind, entity_id)` for `tenant_id`.
    pub async fn freeze(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        tenant_id: Uuid,
        kind: &str,
        entity_id: Uuid,
        version: i64,
    ) {
        let model = ActiveModel {
            tenant_id: Set(tenant_id),
            entity_kind: Set(kind.to_owned()),
            entity_id: Set(entity_id),
            published_version: Set(version),
            content: Set("{}".to_owned()),
            content_digest: Set(vec![0_u8; 32]),
            digest_version: Set(1),
            approval_ref: Set(None),
            actor_ref: Set(ACTOR),
            published_at: Set(Utc.with_ymd_and_hms(2026, 8, 29, 10, 0, 0).unwrap()),
        };
        let conn = provider.conn().expect("scoped connection");
        Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .expect("scope insert")
            .exec_with_returning(&conn)
            .await
            .expect("seed a frozen version row");
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
        /// Raised by the publish door on an override-carrying `bundle` publish,
        /// cleared by slice 06's signal. Admitted in an `UPDATE` only alongside
        /// a `published_version` bump.
        pub composition_pending: bool,
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

    /// Freezes `version` of this product in `products_entity_version`, which
    /// is what the `published_version` guard's existence half looks for. A
    /// probe that bumps `published_version` without calling this is refused
    /// by that clause before the clause it meant to exercise ever runs.
    async fn freeze_version(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        product_id: Uuid,
        version: i64,
    ) {
        super::frozen_version::freeze(provider, scope, TENANT, "product", product_id, version)
            .await;
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
        freeze_version(&provider, &scope, id, 1).await;
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
        freeze_version(&provider, &scope, id, 1).await;

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
        // Freeze the version the jump aims at, so the refusal below can only
        // come from the `+1` clause and not from the existence clause.
        freeze_version(&provider, &scope, id, 2).await;

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
        // Both versions frozen, so only the ordering clause can refuse.
        freeze_version(&provider, &scope, id, 1).await;
        freeze_version(&provider, &scope, id, 2).await;

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
        freeze_version(&provider, &scope, id, 1).await;

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

    /// Class 6b (`published_version`, the existence half): a `+1` bump is
    /// **refused** when `products_entity_version` carries no matching frozen
    /// row.
    ///
    /// This is the half the `DoD` words as "only where the matching frozen
    /// version row exists", owed to Phase 6 until that phase created the
    /// table this clause reads. It is also the empirical proof that a trigger
    /// created by migration `000002` may reference a table created by
    /// migration `000007`, which runs **later** in name order: the chain
    /// booted in `harness()` above, and the clause resolves
    /// `products_entity_version` here, at `UPDATE` time.
    #[tokio::test]
    async fn a_published_version_bump_without_its_version_row_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(30);
        insert(&provider, &scope, draft_row(id, "mike"))
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

        assert!(
            result.is_err(),
            "a published_version bump with no frozen version row behind it is refused"
        );
    }

    /// Class 6b: the positive control for the probe above — the identical
    /// bump, with the matching frozen row seeded first, is admitted.
    ///
    /// Seeded for the **exact** key the clause reads, `(tenant_id,
    /// 'product', product_id, NEW.published_version)`; a row under any other
    /// key would leave this probe passing on a guard that refuses every bump.
    #[tokio::test]
    async fn a_published_version_bump_with_its_version_row_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(31);
        insert(&provider, &scope, draft_row(id, "november"))
            .await
            .expect("insert draft");
        freeze_version(&provider, &scope, id, 1).await;

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

        assert_applied(
            &result,
            "the bump is admitted exactly where the frozen version row exists",
        );
    }

    /// Class 6b: an `UPDATE` that leaves `published_version` unchanged is
    /// admitted with **no** frozen version row anywhere — the existence
    /// clause is gated on the counter moving, so the ordinary edit path pays
    /// nothing for it.
    #[tokio::test]
    async fn an_unchanged_published_version_needs_no_version_row() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(32);
        let mut row = draft_row(id, "oscar");
        row.published_version = Set(3);
        row.lifecycle_state = Set("published".to_owned());
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
            "an edit that does not move published_version is untouched by the existence clause",
        );
    }

    /// Class 6c (`published_version`, the terminal half): a `+1` bump on a
    /// `retired` head is refused, even with its frozen version row in place.
    ///
    /// The clause exists because none of its neighbours reaches this write.
    /// A publish of an already-terminal head writes no `lifecycle_state`, so
    /// the edge clause never fires; the `+1` clause is satisfied; the
    /// existence clause is satisfied by the frozen row seeded below; and
    /// bucket-i and bucket-iii guard columns this update does not touch. So
    /// without a clause of its own the physical layer admitted publishing a
    /// terminal entity, against `cpt-cf-bss-products-dod-transition-guard`'s
    /// "any head write on a `retired` or `discarded` row" (P-D-25, P-D-32)
    /// and against `design/01-foundation.md` §1.6 C5's physical append-only
    /// posture.
    #[tokio::test]
    async fn a_published_version_bump_on_a_retired_head_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(33);
        let mut row = draft_row(id, "papa");
        row.published_version = Set(1);
        row.lifecycle_state = Set("retired".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        // Frozen, so the existence clause cannot be what refuses this.
        freeze_version(&provider, &scope, id, 2).await;

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

        assert!(
            result.is_err(),
            "a terminal head admits no publish: ENTITY_TERMINAL is refused physically, not only by the door"
        );
    }

    /// Class 6c: the same probe against a `discarded` head — the other
    /// terminal state, which the clause names separately.
    #[tokio::test]
    async fn a_published_version_bump_on_a_discarded_head_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(34);
        let mut row = draft_row(id, "quebec");
        row.published_version = Set(1);
        row.lifecycle_state = Set("discarded".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        freeze_version(&provider, &scope, id, 2).await;

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

        assert!(
            result.is_err(),
            "discarded is terminal too, and the clause names both states"
        );
    }

    /// Class 6c: the positive control — the identical bump on a
    /// `deprecated` head is admitted.
    ///
    /// `deprecated` is the last non-terminal state, and a re-publish from it
    /// is an ordinary act (`inst-fd-publish-freeze`). Without this the two
    /// refusals above would pass on a clause that refused every bump.
    #[tokio::test]
    async fn a_published_version_bump_on_a_deprecated_head_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(35);
        let mut row = draft_row(id, "romeo");
        row.published_version = Set(1);
        row.lifecycle_state = Set("deprecated".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        freeze_version(&provider, &scope, id, 2).await;

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

        assert_applied(
            &result,
            "a re-publish from deprecated is an ordinary act, and the terminal clause must not reach it",
        );
    }

    /// Class 6c: the clause is gated on `published_version` **moving**, not
    /// on the row being terminal — an update that leaves the counter alone
    /// is admitted on a terminal head.
    ///
    /// This is the boundary the clause must not overrun. Slice 04 writes
    /// `deprecation_provenance` and `replaced_by_sku_id` **on** terminal rows
    /// by design, so a blanket "no UPDATE on a terminal row" would collide
    /// with the slice that brings those two columns. This probe fails the day
    /// someone simplifies the clause that way.
    #[tokio::test]
    async fn an_update_that_moves_no_counter_is_admitted_on_a_retired_head() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let id = Uuid::from_u128(36);
        let mut row = draft_row(id, "sierra");
        row.published_version = Set(1);
        row.lifecycle_state = Set("retired".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            id,
            vec![
                (Column::UpdatedAt, at(12).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a terminal row still takes writes that move no counter; slice 04 depends on it",
        );
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
            composition_pending: Set(false),
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

    /// Freezes `version` of this SKU in `products_entity_version` — the
    /// `'sku'` half of the same existence clause the sibling module seeds
    /// for products.
    async fn freeze_version(
        provider: &DBProvider<DbError>,
        scope: &AccessScope,
        sku_id: Uuid,
        version: i64,
    ) {
        super::frozen_version::freeze(provider, scope, TENANT, "sku", sku_id, version).await;
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
        freeze_version(&provider, &scope, sku_id, 1).await;
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
        // Freeze the version the jump aims at, so only the `+1` clause can
        // refuse it.
        freeze_version(&provider, &scope, jump_id, 2).await;
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
        // Both versions frozen, so only the ordering clause can refuse.
        freeze_version(&provider, &scope, decrement_id, 1).await;
        freeze_version(&provider, &scope, decrement_id, 2).await;
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
        freeze_version(&provider, &scope, sku_id, 1).await;

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

    /// Class 6b (`published_version`, the existence half): a `+1` bump on a
    /// SKU is refused without its frozen `products_entity_version` row, and
    /// admitted with it.
    ///
    /// The `'sku'` arm of the clause is asserted separately from the
    /// `'product'` arm rather than inferred from it: the two triggers carry
    /// their own `entity_kind` literal and their own primary-key column, so a
    /// copy-paste that left `'product'` in this file would pass every
    /// product-side probe and refuse every SKU publish forever.
    #[tokio::test]
    async fn a_published_version_bump_needs_its_version_row() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(30);
        insert_parent(&provider, &scope, product_id).await;

        let bare_id = Uuid::from_u128(130);
        insert(&provider, &scope, draft_row(bare_id, product_id, "sku-30"))
            .await
            .expect("insert draft sku");
        let refused = update(
            &provider,
            &scope,
            bare_id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await;
        assert!(
            refused.is_err(),
            "a SKU published_version bump with no frozen version row behind it is refused"
        );

        let frozen_id = Uuid::from_u128(131);
        insert(
            &provider,
            &scope,
            draft_row(frozen_id, product_id, "sku-31"),
        )
        .await
        .expect("insert draft sku");
        freeze_version(&provider, &scope, frozen_id, 1).await;
        let admitted = update(
            &provider,
            &scope,
            frozen_id,
            vec![
                (Column::PublishedVersion, 1i64.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
            ],
        )
        .await;
        assert_applied(
            &admitted,
            "the SKU bump is admitted exactly where the frozen version row exists",
        );
    }

    /// Class 6b: an `UPDATE` that leaves `published_version` unchanged is
    /// admitted with no frozen version row at all.
    #[tokio::test]
    async fn an_unchanged_published_version_needs_no_version_row() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(31);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(132);
        let mut row = draft_row(sku_id, product_id, "sku-32");
        row.published_version = Set(3);
        row.lifecycle_state = Set("published".to_owned());
        insert(&provider, &scope, row).await.expect("insert sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::RegionScope, "eu".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "an edit that does not move published_version is untouched by the existence clause",
        );
    }

    /// Class 6c (`published_version`, the terminal half): a `+1` bump on a
    /// `retired` SKU head is refused, with its frozen version row in place.
    ///
    /// The sibling table's probe carries the full rationale; this one
    /// measures the `products_sku` trigger, which states the clause
    /// independently and could drift from it.
    #[tokio::test]
    async fn a_published_version_bump_on_a_retired_head_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(33);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(133);
        let mut row = draft_row(sku_id, product_id, "sku-33");
        row.published_version = Set(1);
        row.lifecycle_state = Set("retired".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        freeze_version(&provider, &scope, sku_id, 2).await;

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::PublishedVersion, 2i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "a terminal SKU head admits no publish either"
        );
    }

    /// Class 6c: the same probe against a `discarded` SKU head.
    #[tokio::test]
    async fn a_published_version_bump_on_a_discarded_head_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(34);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(134);
        let mut row = draft_row(sku_id, product_id, "sku-34");
        row.published_version = Set(1);
        row.lifecycle_state = Set("discarded".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        freeze_version(&provider, &scope, sku_id, 2).await;

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::PublishedVersion, 2i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert!(result.is_err(), "discarded is terminal on this table too");
    }

    /// Class 6c: the positive control — the identical bump on a
    /// `deprecated` SKU head is admitted.
    #[tokio::test]
    async fn a_published_version_bump_on_a_deprecated_head_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(35);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(135);
        let mut row = draft_row(sku_id, product_id, "sku-35");
        row.published_version = Set(1);
        row.lifecycle_state = Set("deprecated".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");
        freeze_version(&provider, &scope, sku_id, 2).await;

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::PublishedVersion, 2i64.into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a re-publish from deprecated is an ordinary act on a SKU as well",
        );
    }

    /// Class 6c: the boundary — an update that moves no counter is admitted
    /// on a terminal SKU head, because slice 04 writes `replaced_by_sku_id`
    /// on exactly such rows.
    #[tokio::test]
    async fn an_update_that_moves_no_counter_is_admitted_on_a_retired_head() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(36);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(136);
        let mut row = draft_row(sku_id, product_id, "sku-36");
        row.published_version = Set(1);
        row.lifecycle_state = Set("retired".to_owned());
        insert(&provider, &scope, row).await.expect("insert row");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::UpdatedAt, at(12).into()),
                (Column::InternalRevision, 2i64.into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "a terminal SKU row still takes writes that move no counter",
        );
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

    /// Class 8 (`composition_pending`): the refusal side. A change to the flag
    /// **without** a `published_version` bump in the same statement is refused
    /// (`design/01-foundation.md` §4.2, the flag is *"changed only in the same
    /// statement as a `published_version` bump"*, P-D-32).
    #[tokio::test]
    async fn a_composition_pending_change_without_a_publish_bump_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(12);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(114);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-14"))
            .await
            .expect("insert draft sku");

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::CompositionPending, true.into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::UpdatedAt, at(12).into()),
            ],
        )
        .await;

        assert!(
            result.is_err(),
            "composition_pending moves only in the statement that bumps published_version"
        );
    }

    /// Class 8 (`composition_pending`): the positive control. The same flag
    /// change **with** a `published_version` bump in the same statement is
    /// admitted — the publish door's own head-row `UPDATE` (P-D-32).
    ///
    /// The bump has two prerequisites of its own that have nothing to do with
    /// this clause: the matching `products_entity_version` row must already
    /// exist, and the head must be non-terminal. The fixture satisfies both,
    /// so a refusal here could only come from the clause under test.
    #[tokio::test]
    async fn a_composition_pending_change_with_a_publish_bump_is_admitted() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(13);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(115);
        insert(&provider, &scope, draft_row(sku_id, product_id, "sku-15"))
            .await
            .expect("insert draft sku");
        freeze_version(&provider, &scope, sku_id, 1).await;

        let result = update(
            &provider,
            &scope,
            sku_id,
            vec![
                (Column::CompositionPending, true.into()),
                (Column::PublishedVersion, 1i64.into()),
                (Column::LifecycleState, "published".to_owned().into()),
                (Column::InternalRevision, 2i64.into()),
                (Column::UpdatedAt, at(12).into()),
            ],
        )
        .await;

        assert_applied(
            &result,
            "the publish door's own head-row UPDATE may raise composition_pending",
        );
    }

    /// The column exists on the executed `SQLite` schema and carries its
    /// default: an `INSERT` that never mentions `composition_pending` stores
    /// the unraised state (**P-D-35**, `NOT NULL` default `false`).
    #[tokio::test]
    async fn composition_pending_defaults_to_false_when_the_insert_omits_it() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let product_id = Uuid::from_u128(14);
        insert_parent(&provider, &scope, product_id).await;
        let sku_id = Uuid::from_u128(116);
        let mut row = draft_row(sku_id, product_id, "sku-16");
        row.composition_pending = sea_orm::ActiveValue::NotSet;

        let stored = insert(&provider, &scope, row)
            .await
            .expect("an INSERT that omits composition_pending is accepted");

        assert!(
            !stored.composition_pending,
            "composition_pending's default is the unraised state"
        );
    }
}

/// `products_entity_version`'s own shape and its frozen-row guard
/// (`cpt-cf-bss-products-dod-version-history-table`), exercised against the
/// executed `SQLite` mirror.
///
/// Unlike the five modules above, these tests speak through the gear's own
/// `entity::entity_version` rather than a test-only mirror, that entity
/// having landed with the migration.
///
/// **The Postgres half is not executed by any test in this file.** The whole
/// suite runs on the in-memory `SQLite` mirror, so the `PL/pgSQL` function
/// and its trigger were compared to the `SQLite` triggers **clause for clause
/// by reading**, not by execution: same two refusals, same two messages, same
/// `BEFORE DELETE OR UPDATE` reach. The same is true of the head-row guard's
/// new existence clause asserted in the two modules above.
///
/// **One §5 probe is owed and cannot run here.** §5 requires that deleting a
/// row a `products_catalog_version_entry` still references be refused by the
/// guard rather than skipped by the GC. That table is slice 06's and does not
/// exist at this commit, so the probe's premise cannot be established. What
/// is asserted instead is the interim rule as shipped — `DELETE` refused
/// unconditionally — which is strictly stronger than the predicate it stands
/// in for; the owed probe lands with the predicate it exercises.
mod entity_version_guard_tests {
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
    use crate::infra::storage::entity::entity_version::{ActiveModel, Column, Entity, Model};

    const TENANT: Uuid = Uuid::from_u128(0xe0_11);
    const ACTOR: Uuid = Uuid::from_u128(0xe0_22);
    const APPROVAL: Uuid = Uuid::from_u128(0xe0_33);

    /// A pinned in-memory `SQLite` pool, one connection only — the identical
    /// idiom every other guard-test module in this file uses, for the
    /// identical reason.
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

    /// The canonical rendering a freeze stores: `JSON`, keys sorted
    /// lexicographically, absent values written `null` rather than omitted,
    /// no insignificant whitespace (§4.3). Held as one literal here so the
    /// round-trip below compares what was written against what came back
    /// rather than against a second literal that could drift from it.
    ///
    /// `1.10` is deliberate. It is a numeric literal Postgres `jsonb` would
    /// have normalized to `1.1` and `text` keeps verbatim, so a future change
    /// of the column type in the direction the digest cannot survive shows up
    /// as a failure of `every_column_the_design_names_round_trips` rather than
    /// as a silent restore-drill failure years later. **On the executed
    /// `SQLite` mirror the column is plain `text` and would keep it either
    /// way** — the assertion is a tripwire for the Postgres statement, which
    /// no test in this file executes. The Postgres column is `text` as well;
    /// see the migration's module doc for why neither `json` nor `jsonb`
    /// could hold this value.
    fn canonical_content() -> String {
        r#"{"brandId":"b-1","name":"alpha","productCode":null,"weight":1.10}"#.to_owned()
    }

    /// A well-formed frozen row: every column set, including the two nullable
    /// ones, so a round-trip reads back what was written.
    fn frozen_row(kind: &str, entity_id: Uuid, version: i64) -> ActiveModel {
        ActiveModel {
            tenant_id: Set(TENANT),
            entity_kind: Set(kind.to_owned()),
            entity_id: Set(entity_id),
            published_version: Set(version),
            content: Set(canonical_content()),
            content_digest: Set(vec![7_u8; 32]),
            digest_version: Set(1),
            approval_ref: Set(Some(APPROVAL)),
            actor_ref: Set(ACTOR),
            published_at: Set(Utc.with_ymd_and_hms(2026, 8, 29, 11, 0, 0).unwrap()),
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

    /// Every column §4.3 names survives a write and a read.
    ///
    /// A round-trip rather than a schema dump: `SeaORM` would fail the
    /// `INSERT` outright on a column the migration does not carry, and the
    /// read-back is what proves the values are stored rather than merely
    /// accepted — including `content`, whose whole purpose is to hand back
    /// **exactly** the bytes the digest was taken over.
    #[tokio::test]
    async fn every_column_the_design_names_round_trips() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(1);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert a frozen row");

        let conn = provider.conn().expect("scoped connection");
        let row = Entity::find()
            .secure()
            .scope_with(&scope)
            .and_id(entity_id)
            .expect("resource-scoped find")
            .one(&conn)
            .await
            .expect("read row")
            .expect("row exists");

        assert_eq!(row.tenant_id, TENANT);
        assert_eq!(row.entity_kind, "product");
        assert_eq!(row.entity_id, entity_id);
        assert_eq!(row.published_version, 1);
        assert_eq!(row.content, canonical_content());
        assert_eq!(row.content_digest, vec![7_u8; 32]);
        assert_eq!(row.digest_version, 1);
        assert_eq!(row.approval_ref, Some(APPROVAL));
        assert_eq!(row.actor_ref, ACTOR);
        assert_eq!(
            row.published_at,
            Utc.with_ymd_and_hms(2026, 8, 29, 11, 0, 0).unwrap()
        );
    }

    /// `approval_ref` is the one nullable column, and a row written without
    /// it is accepted — the state every publish is in until slice 05's gate
    /// mints an `ApprovalRecord` to name.
    #[tokio::test]
    async fn approval_ref_is_nullable() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(2);
        let mut model = frozen_row("sku", entity_id, 1);
        model.approval_ref = Set(None);

        let row = insert(&provider, &scope, model)
            .await
            .expect("a frozen row with no approval behind it is accepted");
        assert_eq!(row.approval_ref, None);
    }

    /// The key is `UNIQUE`: a second row on the same
    /// `(tenant_id, entity_kind, entity_id, published_version)` is refused.
    #[tokio::test]
    async fn the_version_key_is_unique() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(3);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert the first freeze");

        let result = insert(&provider, &scope, frozen_row("product", entity_id, 1)).await;

        assert!(
            result.is_err(),
            "one entity cannot have two frozen rows for one published version"
        );
    }

    /// The key's positive control: the same entity's **next** version, and
    /// the same id under the **other** kind, are both distinct keys and both
    /// admitted. Without this the uniqueness probe above could pass on a
    /// table that refused every second insert.
    #[tokio::test]
    async fn a_different_version_or_kind_is_a_different_key() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(4);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert version 1");
        insert(&provider, &scope, frozen_row("product", entity_id, 2))
            .await
            .expect("version 2 of the same entity is a different key");
        insert(&provider, &scope, frozen_row("sku", entity_id, 1))
            .await
            .expect("the same id under the other kind is a different key");
    }

    /// The `entity_kind` roster is closed to exactly `product` and `sku`.
    #[tokio::test]
    async fn the_entity_kind_roster_is_closed() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        insert(
            &provider,
            &scope,
            frozen_row("product", Uuid::from_u128(5), 1),
        )
        .await
        .expect("product is in the roster");
        insert(&provider, &scope, frozen_row("sku", Uuid::from_u128(6), 1))
            .await
            .expect("sku is in the roster");

        let result = insert(
            &provider,
            &scope,
            frozen_row("catalog_version", Uuid::from_u128(7), 1),
        )
        .await;

        assert!(
            result.is_err(),
            "a third entity kind is a migration, never a convention"
        );
    }

    /// Version `0` has no frozen row: it is the unpublished head's counter
    /// value, and the head-row guard's existence clause reads this table only
    /// for a version the head is moving **to**.
    #[tokio::test]
    async fn version_zero_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);

        let result = insert(
            &provider,
            &scope,
            frozen_row("product", Uuid::from_u128(8), 0),
        )
        .await;

        assert!(result.is_err(), "there is no frozen version zero");
    }

    /// `digest_version` starts at `1` (**P-D-33**) and a row claiming a
    /// lower one is refused — the constant is what makes a later bump
    /// checkable, so a `0` would make the check meaningless.
    #[tokio::test]
    async fn digest_version_below_one_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let mut model = frozen_row("product", Uuid::from_u128(9), 1);
        model.digest_version = Set(0);

        let result = insert(&provider, &scope, model).await;

        assert!(
            result.is_err(),
            "digest_version is pinned at 1 and only ever moves up"
        );
    }

    /// **No `UPDATE` path at all, ever** (§4.3). Probed on `content_digest`,
    /// the column a corrupting rewrite would have to touch to go unnoticed,
    /// and there is no positive control to pair it with because there is no
    /// admitted `UPDATE` for one to exercise.
    #[tokio::test]
    async fn an_update_of_a_frozen_row_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(10);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert a frozen row");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::ContentDigest, Expr::value(vec![9_u8; 32]))
            .filter(Condition::all().add(Column::EntityId.eq(entity_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(
            result.is_err(),
            "a frozen row is never mutated; diffs are computed between rows"
        );
    }

    /// An `UPDATE` that changes nothing is refused too — the guard is
    /// unconditional, not a whitelist with an empty admit list, so a
    /// no-op rewrite is not a way past it.
    #[tokio::test]
    async fn an_update_that_rewrites_the_same_value_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(11);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert a frozen row");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::update_many()
            .col_expr(Column::DigestVersion, Expr::value(1_i32))
            .filter(Condition::all().add(Column::EntityId.eq(entity_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(result.is_err(), "the no-UPDATE rule has no no-op carve-out");
    }

    /// `DELETE` is refused unconditionally at this commit.
    ///
    /// §4.3 admits exactly one `DELETE`, under P-D-40's referential
    /// predicate against `products_catalog_version_entry`. That table is
    /// slice 06's; a literal predicate today would find nothing referencing
    /// any row and therefore admit **every** delete, which is fail-open. The
    /// interim rule this asserts is the strictly stronger one, and it is what
    /// slice 06 replaces in this migration file in place.
    #[tokio::test]
    async fn a_delete_of_a_frozen_row_is_refused() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(12);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert a frozen row");

        let conn = provider.conn().expect("scoped connection");
        let result = Entity::delete_many()
            .filter(Condition::all().add(Column::EntityId.eq(entity_id)))
            .secure()
            .scope_with(&scope)
            .exec(&conn)
            .await;

        assert!(
            result.is_err(),
            "no DELETE is admitted here until slice 06 supplies the referential predicate"
        );
    }

    /// The table is tenant-scoped like every other Foundation table: a
    /// neighbour tenant's scope does not see this tenant's frozen rows.
    #[tokio::test]
    async fn a_frozen_row_is_invisible_to_another_tenant() {
        let provider = harness().await;
        let scope = AccessScope::for_tenant(TENANT);
        let entity_id = Uuid::from_u128(13);
        insert(&provider, &scope, frozen_row("product", entity_id, 1))
            .await
            .expect("insert a frozen row");

        let other = AccessScope::for_tenant(Uuid::from_u128(0xe0_99));
        let conn = provider.conn().expect("scoped connection");
        let found = Entity::find()
            .secure()
            .scope_with(&other)
            .and_id(entity_id)
            .expect("resource-scoped find")
            .one(&conn)
            .await
            .expect("read row");

        assert!(found.is_none(), "version history is tenant-scoped");
    }
}

/// The admitted-edge roster, pinned across every copy of it this repository
/// holds.
///
/// The five-edge state machine `features/foundation.md` §4 declares is written
/// out in **five** places, and nothing in the type system holds them equal:
///
/// 1. [`crate::domain::transition::ADMITTED_EDGES`], the application's copy;
/// 2. `m20260829_000002`'s Postgres `PL/pgSQL` clause;
/// 3. `m20260829_000002`'s `SQLite` trigger `WHEN` clause;
/// 4. `m20260829_000003`'s Postgres clause;
/// 5. `m20260829_000003`'s `SQLite` clause.
///
/// A sixth copy is the design set's prose, which is not machine-checkable from
/// here. Every copy below is measured against the first, so an edge added to
/// or dropped from any one of them fails here rather than at a customer.
///
/// **Two measurement routes, each used where it is the stronger one.** The
/// `SQLite` halves are read back out of `sqlite_master` after the chain has
/// booted, so what is compared is the artifact the engine actually holds
/// rather than a string in this repository that may never have been executed.
/// The Postgres halves are read from the migration **source**, because no test
/// in this crate executes a Postgres statement — the whole suite runs
/// in-memory on `SQLite` — so the source text is the only evidence available,
/// and a `const` that no reader can name from outside its module
/// (`PG_UP_STATEMENTS` is private) has to be reached with `include_str!`.
mod lifecycle_roster_tests {
    use std::collections::BTreeSet;

    use bss_products_sdk::models::LifecycleState;
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    use super::Migrator;
    use crate::domain::transition::ADMITTED_EDGES;

    /// The two migration sources, as text. `include_str!` resolves relative to
    /// the file holding the macro, which is this one — `src/infra/storage/`.
    const PRODUCT_SOURCE: &str =
        include_str!("migrations/m20260829_000002_create_products_product.rs");
    const SKU_SOURCE: &str = include_str!("migrations/m20260829_000003_create_products_sku.rs");

    /// A `(from, to)` edge set in the column's own wire spellings, which is
    /// the one vocabulary all five copies share.
    type Edges = BTreeSet<(String, String)>;

    /// The Postgres statement list of a migration source, sliced out by its
    /// two `const` headers.
    ///
    /// Narrower than scanning the whole file on purpose: it keeps a sentence
    /// of module-doc prose from ever counting as an edge, and it keeps the
    /// `DOWN` list out.
    fn pg_section(source: &str) -> &str {
        let start = source
            .find("const PG_UP_STATEMENTS")
            .expect("every migration declares its Postgres UP list");
        let end = source
            .find("const PG_DOWN_STATEMENTS")
            .expect("every migration declares its Postgres DOWN list");
        &source[start..end]
    }

    /// Every `(OLD.lifecycle_state <op> 'x' AND NEW.lifecycle_state <op> 'y')`
    /// pair in `sql`.
    ///
    /// `comparison` is `=` for a `PL/pgSQL` body and `IS` for a `SQLite`
    /// trigger, which is the whole dialect difference between the two
    /// renderings of one clause.
    ///
    /// The requirement that an `OLD` comparison be **immediately followed** by
    /// the matching `NEW` one is what keeps the bucket clauses out of the
    /// result: those spell the state as `OLD.lifecycle_state IN (...)` and
    /// `NOT IN (...)`, neither of which opens with the comparison operator and
    /// neither of which names `NEW.lifecycle_state` at all.
    fn edges_in(sql: &str, comparison: &str) -> Edges {
        let opening = format!("OLD.lifecycle_state {comparison} '");
        let joining = format!(" AND NEW.lifecycle_state {comparison} '");
        let mut found = Edges::new();
        let mut rest = sql;
        while let Some(at) = rest.find(&opening) {
            let after = &rest[at + opening.len()..];
            rest = after;
            let Some(close) = after.find('\'') else { break };
            let from = &after[..close];
            let tail = &after[close + 1..];
            if let Some(pair) = tail.strip_prefix(&joining)
                && let Some(end) = pair.find('\'')
            {
                found.insert((from.to_owned(), pair[..end].to_owned()));
            }
        }
        found
    }

    /// The application's roster, rendered through `LifecycleState::as_str` —
    /// the same method the column's values come from, so the comparison is
    /// against the spellings that actually reach the database.
    fn declared_edges() -> Edges {
        ADMITTED_EDGES
            .iter()
            .map(|(from, to)| (from.as_str().to_owned(), to.as_str().to_owned()))
            .collect()
    }

    /// Boots the whole chain on a fresh in-memory `SQLite` database and hands
    /// back the **executed** text of one trigger, read from `sqlite_master`.
    ///
    /// A raw `SeaORM` connection rather than the `SecureORM` provider the
    /// guard suites use: `DbConn` deliberately exposes no raw-SQL path, and
    /// `sqlite_master` is not a tenant-scoped table. One connection, pinned,
    /// for the reason every harness in this file pins one — a default
    /// `sqlite::memory:` pool hands each checked-out connection its own empty
    /// database.
    async fn executed_trigger(name: &str) -> String {
        let mut opts = sea_orm::ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1).min_connections(1);
        let db = sea_orm::Database::connect(opts)
            .await
            .expect("connect in-memory sqlite");
        Migrator::up(&db, None).await.expect("boot the chain");
        let query = format!(
            "SELECT sql AS v FROM sqlite_master WHERE type = 'trigger' AND name = '{name}'"
        );
        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                query,
            ))
            .await
            .expect("query sqlite_master");
        let row = rows
            .first()
            .expect("the lifecycle-edge trigger reached the engine");
        let text: String = row.try_get("", "v").expect("the trigger's executed text");
        text
    }

    /// All four SQL copies of the edge roster equal the application's.
    #[tokio::test]
    async fn every_copy_of_the_admitted_edge_roster_agrees() {
        let declared = declared_edges();
        assert_eq!(
            declared.len(),
            ADMITTED_EDGES.len(),
            "a duplicate entry in ADMITTED_EDGES would shrink the set and weaken every comparison below"
        );

        let product_postgres = edges_in(pg_section(PRODUCT_SOURCE), "=");
        let sku_postgres = edges_in(pg_section(SKU_SOURCE), "=");
        let product_sqlite = edges_in(
            &executed_trigger("trg_products_product_lifecycle_edge").await,
            "IS",
        );
        let sku_sqlite = edges_in(
            &executed_trigger("trg_products_sku_lifecycle_edge").await,
            "IS",
        );

        assert_eq!(
            product_postgres, declared,
            "products_product's PL/pgSQL edge list has drifted from ADMITTED_EDGES"
        );
        assert_eq!(
            product_sqlite, declared,
            "products_product's executed SQLite trigger has drifted from ADMITTED_EDGES"
        );
        assert_eq!(
            sku_postgres, declared,
            "products_sku's PL/pgSQL edge list has drifted from ADMITTED_EDGES"
        );
        assert_eq!(
            sku_sqlite, declared,
            "products_sku's executed SQLite trigger has drifted from ADMITTED_EDGES"
        );
    }

    /// The `lifecycle_state` `CHECK` roster, on both engines of both tables,
    /// admits exactly the states the enum spells.
    ///
    /// Asserted without restating the five variants here, which would be a
    /// seventh copy of the roster and would drift like the rest. Two
    /// measurements close it from both sides instead: **every** token in the
    /// `CHECK` parses through `LifecycleState::parse`, whose only outputs are
    /// the five variants, so the set can hold no invented state and is at most
    /// five; and every state the machine names appears in it, and the machine
    /// names all five, so it is at least five.
    #[test]
    fn every_check_roster_admits_exactly_the_lifecycle_states() {
        const MARKER: &str = "CHECK (lifecycle_state IN (";

        for (table, source) in [
            ("products_product", PRODUCT_SOURCE),
            ("products_sku", SKU_SOURCE),
        ] {
            let mut rosters: Vec<BTreeSet<String>> = Vec::new();
            let mut rest = source;
            while let Some(at) = rest.find(MARKER) {
                let after = &rest[at + MARKER.len()..];
                rest = after;
                let close = after.find(')').expect("the token list closes");
                rosters.push(
                    after[..close]
                        .split(',')
                        .map(|token| token.trim().trim_matches('\'').to_owned())
                        .collect(),
                );
            }

            assert_eq!(
                rosters.len(),
                2,
                "{table}: one lifecycle_state CHECK per engine, Postgres and SQLite"
            );
            for roster in &rosters {
                for token in roster {
                    assert!(
                        LifecycleState::parse(token).is_some(),
                        "{table}: the CHECK admits {token}, which is not a LifecycleState"
                    );
                }
                for (from, to) in &ADMITTED_EDGES {
                    assert!(
                        roster.contains(from.as_str()),
                        "{table}: the machine takes an edge out of a state the CHECK refuses"
                    );
                    assert!(
                        roster.contains(to.as_str()),
                        "{table}: the machine takes an edge into a state the CHECK refuses"
                    );
                }
            }
            assert_eq!(
                rosters.first(),
                rosters.last(),
                "{table}: the two engines admit different lifecycle_state rosters"
            );
        }
    }
}

/// §5's agreement test: the `BucketRegistry` and §4.2's trigger column classes
/// name the same columns in the same classes.
///
/// **P-D-32 makes this the only thing holding the two together.** The registry
/// is *advisory for the physical layer* — a compile-time Rust map has no read
/// path from a migration-time trigger — so the trigger's column classes stay
/// static DDL and the two statements of one rule can drift silently. Every
/// other test in this file judges one artifact; this module judges the pair.
///
/// **iii and iv are asserted as one combined class**, because the whitelist
/// admits them together: the `trg_*_bucket_iii` trigger and its `PL/pgSQL`
/// twin carry a single predicate for both, so the physical side cannot tell
/// them apart and this test must not pretend it can. Splitting them here
/// would be inventing a distinction the artifact under test does not make.
///
/// **The mechanical and row-identity columns are outside the comparison**, by
/// §5's own words: `lifecycle_state`, `published_version`,
/// `internal_revision`, `composition_pending` and the update timestamp — and,
/// when their slices land, `deprecation_provenance`, `replaced_by_sku_id` and
/// `cloned_from` — together with `tenant_id`, the primary key and
/// `created_by`. They carry no bucket tag, so a bucket comparison has nothing
/// to say about them. The third assertion below is what keeps that exemption
/// from becoming a hole.
///
/// # Neither side is hand-copied
///
/// A third list of column names written out here would be a third artifact,
/// and would drift exactly as the two under test can. So both sides are read:
///
/// - the **executed** `SQLite` triggers and the executed column list, out of
///   `sqlite_master` and `pragma_table_info` after the chain boots — the
///   artifact the engine actually holds;
/// - the Postgres clauses out of the migration **source** through
///   `include_str!`, because no test in this crate executes a Postgres
///   statement. What that half proves is **source agreement, not engine
///   behaviour**: it catches a `PL/pgSQL` clause that stopped naming what its
///   `SQLite` mirror names, and it cannot catch a Postgres engine that
///   disagrees with its own text.
///
/// The clause is located by its **raise message** rather than by a line
/// number or a column name, since the message is the one part of a clause
/// that names the bucket it enforces.
mod bucket_agreement_tests {
    use std::collections::BTreeSet;

    use bss_products_sdk::models::EntityKind;
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    use super::Migrator;
    use crate::domain::bucket::{self, FieldBucket};

    /// The two migration sources, as text. `include_str!` resolves relative to
    /// the file holding the macro, which is this one — `src/infra/storage/`.
    const PRODUCT_SOURCE: &str =
        include_str!("migrations/m20260829_000002_create_products_product.rs");
    const SKU_SOURCE: &str = include_str!("migrations/m20260829_000003_create_products_sku.rs");

    /// The null-safe comparison each engine spells a guarded column change
    /// with. This is the whole dialect difference between the two renderings
    /// of one clause.
    const PG_COMPARISON: &str = "IS DISTINCT FROM";
    /// `SQLite`'s form of the same comparison.
    const SQLITE_COMPARISON: &str = "IS NOT";

    /// The raise messages that name a bucket, and therefore locate its clause.
    const BUCKET_I_RAISE: &str = "bucket-i columns are admitted";
    /// The combined iii/iv clause's message. There is no `bucket-iv` message
    /// anywhere, which is the point: one clause, one raise, two tags.
    const BUCKET_III_RAISE: &str = "bucket-iii columns are admitted";
    /// The message a bucket-ii clause **would** carry. Spelled with the word
    /// `columns` so it cannot match `bucket-iii columns`, of which
    /// `bucket-ii` is a prefix.
    const BUCKET_II_RAISE: &str = "bucket-ii columns";

    /// Each head table, with the entity kind the registry keys by and the
    /// migration source its Postgres half lives in.
    const TABLES: [(EntityKind, &str, &str); 2] = [
        (EntityKind::Product, "products_product", PRODUCT_SOURCE),
        (EntityKind::Sku, "products_sku", SKU_SOURCE),
    ];

    /// Boots the whole chain on a fresh in-memory `SQLite` database.
    ///
    /// One connection, pinned, for the reason every harness in this file pins
    /// one: a default `sqlite::memory:` pool hands each checked-out
    /// connection its own empty database. A raw `SeaORM` connection rather
    /// than the `SecureORM` provider the guard suites use, because
    /// `sqlite_master` is not a tenant-scoped table and `DbConn` exposes no
    /// raw-SQL path.
    async fn booted() -> sea_orm::DatabaseConnection {
        let mut opts = sea_orm::ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1).min_connections(1);
        let db = sea_orm::Database::connect(opts)
            .await
            .expect("connect in-memory sqlite");
        Migrator::up(&db, None).await.expect("boot the chain");
        db
    }

    /// Every value of a one-column query aliased `v`, against the executed
    /// database.
    async fn read_column(db: &sea_orm::DatabaseConnection, query: String) -> Vec<String> {
        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                query,
            ))
            .await
            .expect("query the executed sqlite schema");
        rows.iter()
            .map(|row| row.try_get("", "v").expect("the queried value"))
            .collect()
    }

    /// The executed text of every trigger the engine holds for `table`.
    async fn executed_trigger_texts(db: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
        read_column(
            db,
            format!(
                "SELECT sql AS v FROM sqlite_master WHERE type = 'trigger' AND tbl_name = '{table}'"
            ),
        )
        .await
    }

    /// The name of every trigger the engine holds for `table`.
    async fn executed_trigger_names(db: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
        read_column(
            db,
            format!(
                "SELECT name AS v FROM sqlite_master WHERE type = 'trigger' AND tbl_name = '{table}'"
            ),
        )
        .await
    }

    /// The executed column list of `table`, read from the engine's own
    /// introspection rather than from the `CREATE TABLE` text this repository
    /// holds.
    async fn executed_columns(db: &sea_orm::DatabaseConnection, table: &str) -> BTreeSet<String> {
        read_column(
            db,
            format!("SELECT name AS v FROM pragma_table_info('{table}')"),
        )
        .await
        .into_iter()
        .collect()
    }

    /// The executed text of the one trigger whose abort message carries
    /// `raise_marker`.
    ///
    /// Located by message, not by name: a renamed trigger still enforces its
    /// bucket, and a trigger that stopped naming its bucket is a finding this
    /// helper turns into a failure rather than hiding behind an empty set.
    fn trigger_raising(texts: &[String], raise_marker: &str) -> String {
        let mut matched = texts.iter().filter(|text| text.contains(raise_marker));
        let found = matched
            .next()
            .expect("the executed schema carries a trigger for this bucket")
            .clone();
        assert!(
            matched.next().is_none(),
            "two executed triggers raise the same bucket message, so neither is the whitelist"
        );
        found
    }

    /// The Postgres statement list of a migration source, sliced out by its
    /// two `const` headers, so no sentence of module-doc prose and no `DOWN`
    /// statement can be read as a clause.
    fn pg_section(source: &str) -> &str {
        let start = source
            .find("const PG_UP_STATEMENTS")
            .expect("every migration declares its Postgres UP list");
        let end = source
            .find("const PG_DOWN_STATEMENTS")
            .expect("every migration declares its Postgres DOWN list");
        &source[start..end]
    }

    /// The `PL/pgSQL` clause that ends in `raise_marker`: the text from the
    /// `IF ` that opens it to the message that names its bucket.
    fn pg_clause<'source>(source: &'source str, raise_marker: &str) -> &'source str {
        let section = pg_section(source);
        let at = section
            .find(raise_marker)
            .expect("the PL/pgSQL body carries a clause for this bucket");
        let head = &section[..at];
        let start = head
            .rfind("IF ")
            .expect("a raise inside a PL/pgSQL body is opened by an IF");
        &head[start..]
    }

    /// Every column `sql` guards as `NEW.x <comparison> OLD.x`.
    ///
    /// The pair form is what makes this a column **class** read rather than a
    /// word count: the state operands a clause also carries are spelled
    /// `OLD.published_version = 0` and `OLD.lifecycle_state IN (...)`, neither
    /// of which names `NEW` at all, so neither can be mistaken for a guarded
    /// column. The trailing boundary check is what keeps
    /// `NEW.name IS NOT OLD.name_normalized` from being read as a guard on
    /// `name`.
    fn compared_columns(sql: &str, comparison: &str) -> BTreeSet<String> {
        const NEW: &str = "NEW.";
        let mut found = BTreeSet::new();
        let mut rest = sql;
        while let Some(at) = rest.find(NEW) {
            let after = &rest[at + NEW.len()..];
            rest = after;
            let end = after.find(is_not_ident).unwrap_or(after.len());
            let column = &after[..end];
            if column.is_empty() {
                continue;
            }
            let expected = format!(" {comparison} OLD.{column}");
            if let Some(tail) = after[end..].strip_prefix(&expected)
                && (tail.is_empty() || tail.starts_with(is_not_ident))
            {
                found.insert(column.to_owned());
            }
        }
        found
    }

    /// Every column `sql` names at all, on either side of an update.
    ///
    /// Deliberately looser than [`compared_columns`]: the third assertion asks
    /// only whether the whitelist has *heard of* a column, so a mention under
    /// any predicate counts.
    fn mentioned_columns(sql: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        for prefix in ["NEW.", "OLD."] {
            let mut rest = sql;
            while let Some(at) = rest.find(prefix) {
                let after = &rest[at + prefix.len()..];
                rest = after;
                let end = after.find(is_not_ident).unwrap_or(after.len());
                if end > 0 {
                    found.insert(after[..end].to_owned());
                }
            }
        }
        found
    }

    /// Whether `c` ends a `SQL` identifier.
    fn is_not_ident(c: char) -> bool {
        !c.is_ascii_alphanumeric() && c != '_'
    }

    /// The registry's columns for a set of tags, as the physical side spells
    /// them.
    ///
    /// Takes a **set** of tags rather than one because §5's combined iii/iv
    /// class is the operand, not a convenience.
    fn registry_bucket(kind: EntityKind, wanted: &[FieldBucket]) -> BTreeSet<String> {
        bucket::columns(kind)
            .iter()
            .filter(|tag| {
                tag.class
                    .bucket()
                    .is_some_and(|tagged| wanted.contains(&tagged))
            })
            .map(|tag| tag.column.to_owned())
            .collect()
    }

    /// Every column the registry names for `kind`, whatever class it puts it
    /// in.
    fn registry_columns(kind: EntityKind) -> BTreeSet<String> {
        bucket::columns(kind)
            .iter()
            .map(|tag| tag.column.to_owned())
            .collect()
    }

    /// A set, as a message an operator can read. `use_debug` is denied here
    /// and a set printed through `Debug` would be the only thing a failure
    /// showed.
    fn listed(columns: &BTreeSet<String>) -> String {
        if columns.is_empty() {
            return "(none)".to_owned();
        }
        columns
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// **Assertion 1.** Bucket-i, and bucket-iii/iv as one combined class,
    /// name the same columns in the registry as in the whitelist, per entity.
    ///
    /// What it catches: a column re-tagged in the registry without its trigger
    /// clause moving, a column added to a clause without a registry row, and
    /// either of those in reverse. Both of those are P-D-32's drift, and until
    /// this test nothing measured it — `bucket_tests` compares the registry
    /// against the **entity model**, which knows a table's columns and nothing
    /// about which class each is guarded in.
    ///
    /// The registry sides are asserted non-empty first, because two empty sets
    /// are equal and a comparison that both artifacts stopped populating would
    /// otherwise pass loudest of all.
    #[tokio::test]
    async fn the_registry_and_the_whitelist_name_the_same_columns_in_the_same_classes() {
        let db = booted().await;
        for (kind, table, source) in TABLES {
            let texts = executed_trigger_texts(&db, table).await;

            let structural_registry = registry_bucket(kind, &[FieldBucket::Structural]);
            let material_registry = registry_bucket(
                kind,
                &[FieldBucket::MaterialMutable, FieldBucket::Descriptive],
            );
            assert!(
                !structural_registry.is_empty(),
                "{table}: the registry tags no bucket-i column, so the comparison below is vacuous"
            );
            assert!(
                !material_registry.is_empty(),
                "{table}: the registry tags no bucket-iii/iv column, so the comparison below is vacuous"
            );

            let structural_executed =
                compared_columns(&trigger_raising(&texts, BUCKET_I_RAISE), SQLITE_COMPARISON);
            let material_executed = compared_columns(
                &trigger_raising(&texts, BUCKET_III_RAISE),
                SQLITE_COMPARISON,
            );
            let structural_source =
                compared_columns(pg_clause(source, BUCKET_I_RAISE), PG_COMPARISON);
            let material_source =
                compared_columns(pg_clause(source, BUCKET_III_RAISE), PG_COMPARISON);

            assert_eq!(
                structural_executed,
                structural_registry,
                "{table}: the executed SQLite bucket-i clause guards [{}], the registry tags [{}]",
                listed(&structural_executed),
                listed(&structural_registry)
            );
            assert_eq!(
                material_executed,
                material_registry,
                "{table}: the executed SQLite bucket-iii/iv clause guards [{}], the registry tags [{}]",
                listed(&material_executed),
                listed(&material_registry)
            );
            assert_eq!(
                structural_source,
                structural_registry,
                "{table}: the PL/pgSQL bucket-i clause guards [{}], the registry tags [{}]",
                listed(&structural_source),
                listed(&structural_registry)
            );
            assert_eq!(
                material_source,
                material_registry,
                "{table}: the PL/pgSQL bucket-iii/iv clause guards [{}], the registry tags [{}]",
                listed(&material_source),
                listed(&material_registry)
            );
        }
    }

    /// **Assertion 2.** Bucket-ii has no members, in either artifact.
    ///
    /// §4.2's interim row-image predicate for the class is *"only while
    /// `published_version = 0` and `lifecycle_state` is non-terminal"*,
    /// through `inst-fd-save-txn` (P-D-41), and after first publish *"only in
    /// the same statement as a `published_version` bump"* (P-D-34), with a
    /// **tighter predicate still owed by slice 07**. No Foundation column
    /// carries the tag today, so what that predicate has to say about this
    /// gear's columns is exactly nothing — and the honest assertion is that
    /// **both** sides say nothing, not that the arm is skipped.
    ///
    /// Asserted rather than skipped because the two ways this could go wrong
    /// are opposite and both silent: a column tagged bucket-ii in the registry
    /// with no clause to enforce it (a bucket the door routes by and the
    /// database does not know), or a bucket-ii clause installed with no
    /// registry row (a refusal the door cannot predict). When 07 supplies the
    /// tighter predicate this test is **re-pointed at it** — the emptiness
    /// arms below become a membership comparison in the shape of assertion 1,
    /// against 07's clause rather than against nothing.
    #[tokio::test]
    async fn bucket_ii_has_no_members_in_either_artifact() {
        let db = booted().await;
        for (kind, table, source) in TABLES {
            let correctable = registry_bucket(kind, &[FieldBucket::Correctable]);
            assert!(
                correctable.is_empty(),
                "{table}: the registry tags [{}] bucket-ii, and no trigger clause enforces the class",
                listed(&correctable)
            );

            assert!(
                !pg_section(source).contains(BUCKET_II_RAISE),
                "{table}: the PL/pgSQL body carries a bucket-ii clause the registry tags no column for"
            );
            for text in executed_trigger_texts(&db, table).await {
                assert!(
                    !text.contains(BUCKET_II_RAISE),
                    "{table}: an executed trigger carries a bucket-ii clause the registry tags no column for"
                );
            }
            let named = format!("trg_{table}_bucket_ii");
            for name in executed_trigger_names(&db, table).await {
                assert_ne!(
                    name, named,
                    "{table}: a bucket-ii trigger is installed and the registry tags no column for it"
                );
            }
        }
    }

    /// **Assertion 3 (P-D-50).** No published-state column is named by
    /// *neither* artifact.
    ///
    /// This is the case the first two are blind to **by construction**: a
    /// column absent from the registry *and* absent from every trigger clause
    /// is in no bucket set on either side, so every set comparison above holds
    /// trivially while the column sits on the table unclassified. At runtime
    /// that column is exactly what the registry's fail-closed miss refuses —
    /// `classify` answers `ILLEGAL_FIELD_MUTATION` rather than routing it to a
    /// default bucket, which turns an unnoticed schema addition into a refused
    /// save. This test is what turns it into a red build instead.
    ///
    /// The population is the **executed** column list, so a column added to
    /// the table is in the population the moment the migration runs, whether
    /// or not anyone remembered the registry. Being named by *either* artifact
    /// is enough: `updated_at` is mechanical and no clause guards it, and the
    /// registry names it; the immutable set is named by both.
    #[tokio::test]
    async fn no_published_state_column_is_named_by_neither_artifact() {
        let db = booted().await;
        for (kind, table, _) in TABLES {
            let population = executed_columns(&db, table).await;
            assert!(
                !population.is_empty(),
                "{table}: no executed columns were read, so this assertion measures nothing"
            );

            let mut whitelisted = BTreeSet::new();
            for text in executed_trigger_texts(&db, table).await {
                whitelisted.extend(mentioned_columns(&text));
            }
            let registered = registry_columns(kind);

            let unclassified: BTreeSet<String> = population
                .into_iter()
                .filter(|column| !registered.contains(column) && !whitelisted.contains(column))
                .collect();
            assert!(
                unclassified.is_empty(),
                "{table}: [{}] sit on the table and are named by neither the BucketRegistry nor the trigger whitelist, so the save door would refuse them under P-D-50's fail-closed miss",
                listed(&unclassified)
            );
        }
    }
}

/// The two `2026-09-01` tables, probed the way every guard suite above probes
/// its own: boot the chain on a pinned in-memory `SQLite`, then arm each raw
/// probe against the exact CHECK it exists to prove.
///
/// @cpt-dod:cpt-cf-bss-products-dod-version-counter:p1
/// @cpt-dod:cpt-cf-bss-products-dod-watermark-tables:p1
mod counter_and_watermark_guard_tests {
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    use super::Migrator;

    /// A pinned in-memory `SQLite` database with the whole chain applied —
    /// the identical idiom every other guard-test module in this file uses,
    /// raw because these tables have no entity models yet.
    async fn harness() -> sea_orm::DatabaseConnection {
        let mut opts = sea_orm::ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1).min_connections(1);
        let db = sea_orm::Database::connect(opts)
            .await
            .expect("connect in-memory sqlite");
        Migrator::up(&db, None).await.expect("boot the chain");
        db
    }

    async fn exec(db: &sea_orm::DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
        db.execute_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .map(|_| ())
    }

    /// The counter's floor CHECK: `next_id` at the pinned start of `1` is
    /// admitted, and the poison value `0` — an id below the P-D-67 start —
    /// is refused by name.
    #[tokio::test]
    async fn the_counter_floor_admits_one_and_refuses_zero() {
        let db = harness().await;
        exec(
            &db,
            "INSERT INTO products_catalog_version_counter (tenant_id, next_id) VALUES ('t-a', 1)",
        )
        .await
        .expect("next_id = 1 is the pinned start and must be admitted");

        let err = exec(
            &db,
            "INSERT INTO products_catalog_version_counter (tenant_id, next_id) VALUES ('t-b', 0)",
        )
        .await
        .expect_err("next_id = 0 sits below the pinned start and must be refused");
        assert!(
            err.to_string()
                .contains("chk_products_catalog_version_counter_floor"),
            "the refusal must come from the floor CHECK, not from some other guard: {err}"
        );
    }

    /// The watermark's hash CHECK: a 64-char lowercase hex digest is
    /// admitted; a truncated digest — the value that would silently never
    /// match again at the idempotence comparison — is refused by name.
    #[tokio::test]
    async fn the_watermark_hash_check_pins_sixty_four_lowercase_hex() {
        let db = harness().await;
        let good = "a".repeat(64);
        exec(
            &db,
            &format!(
                "INSERT INTO products_reference_watermark \
                 (tenant_id, producer, watermark_at, posted_at, set_hash) \
                 VALUES ('t-a', 'plan-price', '2026-09-01T00:00:00Z', '2026-09-01T00:00:01Z', '{good}')"
            ),
        )
        .await
        .expect("a 64-char lowercase hex digest is the stored shape");

        for (label, bad) in [
            ("truncated", "a".repeat(63)),
            ("upper-cased", "A".repeat(64)),
        ] {
            let err = exec(
                &db,
                &format!(
                    "INSERT INTO products_reference_watermark \
                     (tenant_id, producer, watermark_at, posted_at, set_hash) \
                     VALUES ('t-b', 'plan-price', '2026-09-01T00:00:00Z', '2026-09-01T00:00:01Z', '{bad}')"
                ),
            )
            .await
            .expect_err("a digest outside the pinned shape must be refused");
            assert!(
                err.to_string()
                    .contains("chk_products_reference_watermark_hash_len"),
                "{label}: the refusal must come from the hash CHECK: {err}"
            );
        }
    }

    /// `never-received` is the absence of the watermark row (`P-D-71`): the
    /// chain seeds nothing into either watermark table, so a freshly booted
    /// database has zero rows — a sentinel row here would be the poison-value
    /// arm the decision declined.
    #[tokio::test]
    async fn the_chain_seeds_no_watermark_rows() {
        let db = harness().await;
        for table in ["products_reference_watermark", "products_reference_member"] {
            let rows = db
                .query_all_raw(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!("SELECT count(*) AS v FROM {table}"),
                ))
                .await
                .expect("count the table");
            let n: i64 = rows.first().expect("one row").try_get("", "v").expect("v");
            assert_eq!(n, 0, "{table} must be born empty");
        }
    }

    /// The member table's empty-producer CHECK holds on both tables' shared
    /// convention, and its per-SKU index exists — the membership lookup the
    /// `DoD` requires to be an index hit rides it.
    #[tokio::test]
    async fn the_member_table_refuses_an_empty_producer_and_carries_the_sku_index() {
        let db = harness().await;
        let err = exec(
            &db,
            "INSERT INTO products_reference_member (tenant_id, producer, sku_id) \
             VALUES ('t-a', '', 's-1')",
        )
        .await
        .expect_err("an empty producer name must be refused");
        assert!(
            err.to_string()
                .contains("chk_products_reference_member_producer"),
            "the refusal must come from the producer CHECK: {err}"
        );

        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT name AS v FROM sqlite_master WHERE type = 'index' \
                 AND name = 'idx_products_reference_member_sku'"
                    .to_owned(),
            ))
            .await
            .expect("query sqlite_master");
        assert_eq!(rows.len(), 1, "the per-SKU membership index must exist");
    }
}

/// `products_catalog_version`'s whitelist guard, probed the way the head-row
/// guards above are: every frozen column poked one at a time, the one
/// admitted column flipped, and the two refusal messages told apart —
/// `dod-catalog-version-table` requires the delete arm's and the update
/// arm's texts asserted separately, because a body that lost its UPDATE
/// branch would still refuse an update, with the wrong message.
///
/// @cpt-dod:cpt-cf-bss-products-dod-catalog-version-table:p1
mod catalog_version_guard_tests {
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    use super::Migrator;

    async fn harness() -> sea_orm::DatabaseConnection {
        let mut opts = sea_orm::ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1).min_connections(1);
        let db = sea_orm::Database::connect(opts)
            .await
            .expect("connect in-memory sqlite");
        Migrator::up(&db, None).await.expect("boot the chain");
        db
    }

    async fn exec(db: &sea_orm::DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
        db.execute_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .map(|_| ())
    }

    async fn seed_version(db: &sea_orm::DatabaseConnection) {
        exec(
            db,
            "INSERT INTO products_catalog_version \
             (tenant_id, catalog_version_id, checksum, digest_version, published_at, \
              participant_set_snapshot, freeze_state) \
             VALUES ('t-a', 1, 'c1', 1, '2026-09-01T00:00:00Z', '[]', 'open')",
        )
        .await
        .expect("a well-shaped version row is admitted");
    }

    /// `freeze_state` is the one column the UPDATE arm admits, and its own
    /// roster CHECK still governs the value it moves to.
    #[tokio::test]
    async fn freeze_state_is_the_only_admitted_update() {
        let db = harness().await;
        seed_version(&db).await;

        exec(
            &db,
            "UPDATE products_catalog_version SET freeze_state = 'complete' \
             WHERE tenant_id = 't-a' AND catalog_version_id = 1",
        )
        .await
        .expect("the admitted column must move");

        let err = exec(
            &db,
            "UPDATE products_catalog_version SET freeze_state = 'half-done' \
             WHERE tenant_id = 't-a' AND catalog_version_id = 1",
        )
        .await
        .expect_err("a value outside the roster must be refused");
        assert!(
            err.to_string()
                .contains("chk_products_catalog_version_freeze_state"),
            "the roster CHECK still governs the admitted column: {err}"
        );
    }

    /// Every frozen column is poked one at a time, and each refusal carries
    /// the UPDATE arm's message — not the delete arm's.
    #[tokio::test]
    async fn every_frozen_column_is_refused_by_the_update_arm() {
        let db = harness().await;
        seed_version(&db).await;

        for (column, poison) in [
            ("tenant_id", "'t-b'"),
            ("catalog_version_id", "2"),
            ("checksum", "'c2'"),
            ("digest_version", "2"),
            ("published_at", "'2026-09-02T00:00:00Z'"),
            ("participant_set_snapshot", "'[\"plan-price\"]'"),
        ] {
            let err = exec(
                &db,
                &format!(
                    "UPDATE products_catalog_version SET {column} = {poison} \
                     WHERE tenant_id = 't-a' AND catalog_version_id = 1"
                ),
            )
            .await
            .expect_err("a frozen column must be refused");
            let text = err.to_string();
            assert!(
                text.contains("freeze_state is the only column the UPDATE arm admits"),
                "{column}: the refusal must be the UPDATE arm's own message: {text}"
            );
        }
    }

    /// The delete arm refuses with its own message, told apart from the
    /// update arm's by text — the assertion the `DoD` requires separately.
    #[tokio::test]
    async fn delete_is_refused_with_the_delete_arms_own_message() {
        let db = harness().await;
        seed_version(&db).await;

        let err = exec(
            &db,
            "DELETE FROM products_catalog_version \
             WHERE tenant_id = 't-a' AND catalog_version_id = 1",
        )
        .await
        .expect_err("DELETE must be refused outright");
        let text = err.to_string();
        assert!(
            text.contains("append-only: DELETE is not permitted"),
            "the refusal must be the delete arm's: {text}"
        );
        assert!(
            !text.contains("UPDATE arm admits"),
            "the delete refusal must not ride the update arm's message: {text}"
        );
    }

    /// The id floor pins the P-D-67 counter start and the digest floor pins
    /// the P-D-73 companion — each poison refused by its CHECK's name.
    #[tokio::test]
    async fn the_id_and_digest_floors_hold() {
        let db = harness().await;
        for (label, sql, check) in [
            (
                "id zero",
                "INSERT INTO products_catalog_version \
                 (tenant_id, catalog_version_id, checksum, digest_version, published_at, \
                  participant_set_snapshot, freeze_state) \
                 VALUES ('t-b', 0, 'c', 1, '2026-09-01T00:00:00Z', '[]', 'open')",
                "chk_products_catalog_version_id_floor",
            ),
            (
                "digest zero",
                "INSERT INTO products_catalog_version \
                 (tenant_id, catalog_version_id, checksum, digest_version, published_at, \
                  participant_set_snapshot, freeze_state) \
                 VALUES ('t-b', 1, 'c', 0, '2026-09-01T00:00:00Z', '[]', 'open')",
                "chk_products_catalog_version_digest",
            ),
        ] {
            let err = exec(&db, sql)
                .await
                .expect_err("a floor poison must be refused");
            assert!(
                err.to_string().contains(check),
                "{label}: the refusal must come from {check}: {err}"
            );
        }
    }
}
