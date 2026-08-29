//! `products_product` / `products_sku` / `products_identity_ref` repository
//! tests, against the executed `SQLite` mirror.
//!
//! Written before [`super::insert_product`] and its siblings existed, and run
//! red against the stub before the implementation landed, per this phase's own
//! rule: the ordering is required, not stylistic.
//!
//! # The corrupt-row case is exercised on a hand-built `Model`, not through SQL
//!
//! `into_product_record` and `into_sku_record` refuse an unparseable
//! `lifecycle_state`. There is no way to make the database hold one to read
//! back: `chk_products_product_lifecycle_state` and
//! `chk_products_sku_lifecycle_state` are real `CHECK` constraints on the
//! `SQLite` mirror as well as on Postgres, so an `INSERT` outside the
//! enumeration is refused at write time rather than landing — and
//! `toolkit-db`'s `DBRunner` is sealed specifically so that gear code, test
//! code included, can never reach a raw connection to work around that. The
//! only way left to ask "what does this repository do when the table was
//! written around" is to hand `into_product_record` a [`product::Model`] this
//! gear could not itself have inserted, which is exactly what
//! `an_unparseable_lifecycle_state_is_a_corrupt_row` and its `SKU` twin below
//! do. `approval_repo_tests` in the sibling pricing gear takes the identical
//! shortcut for the identical reason.
#![allow(clippy::expect_used)]

use chrono::{TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    AuditCommon, NewProduct, NewSku, RefusalSubject, RepoError, find_product, find_sku,
    insert_product, insert_sku, into_product_record, into_sku_record, resolve_actor_ref,
    write_elevated_read_audit, write_eventless_act_audit, write_refusal_audit,
};
use crate::domain::error::DomainError;
use crate::infra::storage::entity::{audit_log, identity_ref, product, sku};
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const BRAND: Uuid = Uuid::from_u128(0xb1_01);
const PRODUCT: Uuid = Uuid::from_u128(0xf0_01);
const SKU: Uuid = Uuid::from_u128(0x5c_01);
const AUDIT: Uuid = Uuid::from_u128(0xa0_01);
const SESSION: Uuid = Uuid::from_u128(0x5e_01);

/// A pinned in-memory `SQLite` pool, one connection only.
///
/// A default `sqlite::memory:` pool hands each checked-out connection its own
/// empty database, so a pool of more than one connection makes the migrations
/// applied on connection A invisible to a query issued on connection B — the
/// table would appear to vanish. Pinning to one connection is what
/// `chat-engine`'s `stream_event_repo_tests` does for the identical failure
/// mode.
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

fn new_product(product_id: Uuid, tenant_id: Uuid) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id,
        brand_id: BRAND,
        name: "Fibre 500".to_owned(),
        name_normalized: "fibre 500".to_owned(),
        product_code: Some("FIBRE-500".to_owned()),
        region_scope: "eu,apac".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
    }
}

fn new_sku(sku_id: Uuid, tenant_id: Uuid, product_id: Uuid) -> NewSku {
    NewSku {
        sku_id,
        tenant_id,
        product_id,
        sku_code: "FIBRE-500-STD".to_owned(),
        region_scope: "eu".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
    }
}

/// A Product inserted through the repository is read back with every field
/// intact.
///
/// If the insert or the read mapped a single column wrong — swapped
/// `region_scope` and `brand_scope`, dropped `product_code`, forgot to seed
/// `internal_revision` at `1` — this is the only test that would notice,
/// since [`insert_product`] never reads back a value it did not itself just
/// write into the record it returns.
#[tokio::test]
async fn a_product_inserted_through_the_repository_reads_back_with_every_field_intact() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let created = insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let found = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");

    assert_eq!(found, created);
    assert_eq!(found.name, "Fibre 500");
    assert_eq!(found.name_normalized, "fibre 500");
    assert_eq!(found.product_code.as_deref(), Some("FIBRE-500"));
    assert_eq!(
        found.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Draft
    );
    assert_eq!(found.internal_revision, 1);
    assert_eq!(found.published_version, 0);
    assert_eq!(found.region_scope, "eu,apac");
    assert_eq!(found.brand_scope, "");
    assert_eq!(found.created_by, "principal:author-1");
    assert_eq!(found.created_at, at(9));
    assert_eq!(found.updated_at, at(9));
}

/// A SKU inserted through the repository is read back with every field
/// intact, the `SKU` twin of the Product case above.
#[tokio::test]
async fn a_sku_inserted_through_the_repository_reads_back_with_every_field_intact() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert parent product");

    let created = insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");

    let found = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");

    assert_eq!(found, created);
    assert_eq!(found.product_id, PRODUCT);
    assert_eq!(found.sku_code, "FIBRE-500-STD");
    assert_eq!(
        found.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Draft
    );
    assert_eq!(found.internal_revision, 1);
    assert_eq!(found.published_version, 0);
    assert_eq!(found.region_scope, "eu");
    assert_eq!(found.brand_scope, "");
    assert_eq!(found.created_by, "principal:author-1");
    assert_eq!(found.created_at, at(9));
    assert_eq!(found.updated_at, at(9));
}

/// Reading an id that was never inserted answers `Ok(None)`, for both
/// entities — not an error. A caller that cannot tell "absent" from "storage
/// broke" cannot implement the create door's own idempotent-lookup path.
#[tokio::test]
async fn finding_an_id_that_was_never_inserted_answers_none_not_an_error() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    assert_eq!(
        find_product(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read product"),
        None
    );
    assert_eq!(
        find_sku(&conn, &scope, TENANT, SKU)
            .await
            .expect("read sku"),
        None
    );
}

/// A row belonging to another tenant is invisible through a scope for this
/// tenant — the same `Ok(None)` a genuinely absent row answers with, and
/// deliberately so.
///
/// The catalog is commercially sensitive (`fr-identifier-contract`'s
/// isolation neighbour), so a repository that told `OTHER_TENANT` "forbidden"
/// instead of "not found" would have confirmed `PRODUCT` exists under some
/// tenant — an existence leak the SQL-level scope predicate exists to close.
/// An untested boundary is not a boundary, which is why this gets its own
/// case rather than living as a comment on the one above.
#[tokio::test]
async fn a_row_belonging_to_another_tenant_is_not_visible_through_a_foreign_scope() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let owner_scope = AccessScope::for_tenant(TENANT);
    let foreign_scope = AccessScope::for_tenant(OTHER_TENANT);

    insert_product(&conn, &owner_scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    assert_eq!(
        find_product(&conn, &foreign_scope, TENANT, PRODUCT)
            .await
            .expect("scoped read must not error"),
        None,
        "a foreign scope must see exactly what an absent row looks like"
    );
}

/// The `SKU` twin of
/// `a_row_belonging_to_another_tenant_is_not_visible_through_a_foreign_scope`.
///
/// `find_sku`'s own `.secure().scope_with(scope)` call is untested without
/// this case, which is exactly the boundary the Product case's doc argues
/// cannot live as a comment on another test.
#[tokio::test]
async fn a_sku_belonging_to_another_tenant_is_not_visible_through_a_foreign_scope() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let owner_scope = AccessScope::for_tenant(TENANT);
    let foreign_scope = AccessScope::for_tenant(OTHER_TENANT);

    insert_product(&conn, &owner_scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &owner_scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");

    assert_eq!(
        find_sku(&conn, &foreign_scope, TENANT, SKU)
            .await
            .expect("scoped read must not error"),
        None,
        "a foreign scope must see exactly what an absent row looks like"
    );
}

/// A second Product colliding on `(tenant_id, brand_id, name_normalized)`
/// is refused as [`RepoError::Db`] — the documented behaviour
/// [`insert_product`]'s own doc promises for `uq_products_product_name`,
/// asserted here rather than left to the doc comment alone.
#[tokio::test]
async fn a_duplicate_product_name_within_a_tenant_and_brand_is_refused_as_a_db_error() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert first product");

    let second_id = Uuid::from_u128(0xf0_02);
    let err = insert_product(&conn, &scope, new_product(second_id, TENANT))
        .await
        .expect_err("a duplicate name_normalized must be refused");
    assert!(matches!(err, RepoError::Db(_)));
}

/// A second SKU colliding on `(tenant_id, sku_code)` is refused as
/// [`RepoError::Db`] — the documented behaviour [`insert_sku`]'s own doc
/// promises for `uq_products_sku_code`.
#[tokio::test]
async fn a_duplicate_sku_code_within_a_tenant_is_refused_as_a_db_error() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert parent product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert first sku");

    let second_id = Uuid::from_u128(0x5c_02);
    let err = insert_sku(&conn, &scope, new_sku(second_id, TENANT, PRODUCT))
        .await
        .expect_err("a duplicate sku_code must be refused");
    assert!(matches!(err, RepoError::Db(_)));
}

/// A SKU inserted against a `product_id` with no matching Product row is
/// refused as [`RepoError::Db`] — the documented `fk_products_sku_product`
/// path [`insert_sku`]'s own doc promises.
#[tokio::test]
async fn a_sku_referencing_a_nonexistent_product_is_refused_as_a_db_error() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let orphan_product_id = Uuid::from_u128(0xf0_99);
    let err = insert_sku(&conn, &scope, new_sku(SKU, TENANT, orphan_product_id))
        .await
        .expect_err("a sku with no parent product must be refused");
    assert!(matches!(err, RepoError::Db(_)));
}

/// An unparseable `lifecycle_state` on a hand-built [`product::Model`] is
/// read back as [`RepoError::CorruptRow`] — see this file's module doc for
/// why a hand-built model, rather than a written row, is the only way to
/// drive this path.
#[test]
fn an_unparseable_product_lifecycle_state_is_a_corrupt_row() {
    let row = product::Model {
        product_id: PRODUCT,
        tenant_id: TENANT,
        brand_id: BRAND,
        name: "Fibre 500".to_owned(),
        name_normalized: "fibre 500".to_owned(),
        product_code: None,
        lifecycle_state: "paused".to_owned(),
        internal_revision: 1,
        published_version: 0,
        region_scope: String::new(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        updated_at: at(9),
    };

    let err = into_product_record(row).expect_err("an unrecognised token must be refused");
    assert!(matches!(err, RepoError::CorruptRow(ref detail) if detail.contains("paused")));
}

/// The `SKU` twin of `an_unparseable_product_lifecycle_state_is_a_corrupt_row`.
#[test]
fn an_unparseable_sku_lifecycle_state_is_a_corrupt_row() {
    let row = sku::Model {
        sku_id: SKU,
        tenant_id: TENANT,
        product_id: PRODUCT,
        sku_code: "FIBRE-500-STD".to_owned(),
        lifecycle_state: "paused".to_owned(),
        internal_revision: 1,
        published_version: 0,
        region_scope: String::new(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        updated_at: at(9),
    };

    let err = into_sku_record(row).expect_err("an unrecognised token must be refused");
    assert!(matches!(err, RepoError::CorruptRow(ref detail) if detail.contains("paused")));
}

/// Tombstone a live identity-ref row directly, bypassing `resolve_actor_ref`.
///
/// There is no tombstone door in this slice — erasure is slice 10's — so this
/// is the only way a test can put a row into the tombstoned state
/// `resolve_actor_ref` must mint a fresh ref past. It uses the same
/// `update_many().secure().scope_with(...)` chain the repository itself uses,
/// never a raw connection: `DBRunner` deliberately does not implement
/// `SeaORM`'s `ConnectionTrait`, so there is no other way to reach the row.
async fn tombstone(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    at: chrono::DateTime<Utc>,
) {
    identity_ref::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(identity_ref::Column::TombstonedAt, Expr::value(at))
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::ActorRef.eq(actor_ref)),
        )
        .exec(runner)
        .await
        .expect("tombstone the row directly");
}

/// A principal with no ref gets one minted, with `first_seen_at ==
/// last_seen_at` on the freshly minted row.
///
/// If minting ever stamped the two columns from different instants, an
/// age-based erasure computed from `first_seen_at` and one computed from
/// `last_seen_at` would already disagree on a row that has never been
/// resolved a second time.
#[tokio::test]
async fn a_principal_with_no_ref_gets_one_minted_with_first_and_last_seen_equal() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint a fresh ref");

    let row = identity_ref::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(TENANT))
                .add(identity_ref::Column::ActorRef.eq(actor_ref)),
        )
        .one(&conn)
        .await
        .expect("read the minted row")
        .expect("the row exists");

    assert_eq!(row.principal_ref, "principal:alice");
    assert_eq!(row.first_seen_at, at(9));
    assert_eq!(row.last_seen_at, at(9));
    assert_eq!(row.tombstoned_at, None);
}

/// Resolving the same principal again returns the same `actor_ref` and
/// advances `last_seen_at` while leaving `first_seen_at` untouched.
///
/// This is the recorded failure's regression test: an earlier version of
/// this rule advanced `last_seen_at` only at mint time, which pinned it to
/// `first_seen_at` forever and let age-based erasure tombstone an active
/// employee mid-employment. Both halves — the ref staying the same and the
/// two timestamps parting ways — must hold for that failure to be closed.
#[tokio::test]
async fn resolving_the_same_principal_again_returns_the_same_ref_and_advances_last_seen_only() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let first = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint a fresh ref");
    let second = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(15))
        .await
        .expect("resolve the same principal again");

    assert_eq!(
        first, second,
        "the same live principal must resolve to the same ref"
    );

    let row = identity_ref::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(TENANT))
                .add(identity_ref::Column::ActorRef.eq(second)),
        )
        .one(&conn)
        .await
        .expect("read the row")
        .expect("the row exists");

    assert_eq!(
        row.first_seen_at,
        at(9),
        "first_seen_at must never move again"
    );
    assert_eq!(
        row.last_seen_at,
        at(15),
        "last_seen_at must advance on every resolution"
    );
}

/// Two different principals in the same tenant get two different refs.
#[tokio::test]
async fn two_different_principals_in_one_tenant_get_two_different_refs() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let alice = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint alice's ref");
    let bob = resolve_actor_ref(&conn, &scope, TENANT, "principal:bob", at(9))
        .await
        .expect("mint bob's ref");

    assert_ne!(alice, bob);
}

/// The same principal in two different tenants gets two different refs —
/// resolution is tenant-scoped, not global to the principal handle.
#[tokio::test]
async fn the_same_principal_in_two_tenants_gets_two_different_refs() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let other_scope = AccessScope::for_tenant(OTHER_TENANT);

    let ref_in_tenant = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint alice's ref in the first tenant");
    let ref_in_other_tenant =
        resolve_actor_ref(&conn, &other_scope, OTHER_TENANT, "principal:alice", at(9))
            .await
            .expect("mint alice's ref in the second tenant");

    assert_ne!(ref_in_tenant, ref_in_other_tenant);
}

/// After a ref is tombstoned, resolving that principal mints a fresh,
/// different `actor_ref`, and the tombstoned row is still present and still
/// tombstoned.
///
/// A tombstoned ref is retired permanently: every append-only record keeps
/// the `actor_ref` it was stamped with, so reusing the retired key for the
/// same principal would make render-time joins show the new identity against
/// historical rows. Both halves matter — the mint must be fresh, and the old
/// row must survive unmodified, since it is what those historical records
/// still point at.
#[tokio::test]
async fn a_principal_acting_after_its_ref_is_tombstoned_mints_a_fresh_different_ref() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let original = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint the original ref");

    tombstone(&conn, &scope, TENANT, original, at(12)).await;

    let fresh = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(15))
        .await
        .expect("mint a fresh ref past the tombstone");

    assert_ne!(original, fresh, "a retired ref must never be reused");

    let original_row = identity_ref::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(TENANT))
                .add(identity_ref::Column::ActorRef.eq(original)),
        )
        .one(&conn)
        .await
        .expect("read the original row")
        .expect("the tombstoned row must still be present");

    assert_eq!(original_row.tombstoned_at, Some(at(12)));
}

/// Resolution under a foreign `AccessScope` does not see another tenant's
/// ref — the identity-ref twin of
/// `a_row_belonging_to_another_tenant_is_not_visible_through_a_foreign_scope`.
///
/// A repository that let a foreign scope's read fall through to another
/// tenant's live ref would silently misattribute the acting principal's
/// audit trail to the wrong tenant's existing identity — the same existence
/// leak `find_product`'s cross-tenant case exists to close, transplanted to
/// the mint path: the scoped read sees nothing under `TENANT`, and the
/// insert that would otherwise mint a substitute is itself refused because
/// its row's `tenant_id` cannot satisfy a scope for `OTHER_TENANT`.
#[tokio::test]
async fn resolution_under_a_foreign_scope_does_not_see_another_tenants_ref() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let owner_scope = AccessScope::for_tenant(TENANT);
    let foreign_scope = AccessScope::for_tenant(OTHER_TENANT);

    let owners_ref = resolve_actor_ref(&conn, &owner_scope, TENANT, "principal:alice", at(9))
        .await
        .expect("mint the owner's ref");

    let err = resolve_actor_ref(&conn, &foreign_scope, TENANT, "principal:alice", at(9))
        .await
        .expect_err("a foreign scope must neither see nor mint under TENANT's rows");
    assert!(matches!(err, RepoError::Db(_)));

    // The owner's own ref is untouched by the foreign attempt.
    let still_owners_ref =
        resolve_actor_ref(&conn, &owner_scope, TENANT, "principal:alice", at(20))
            .await
            .expect("the owner can still resolve normally");
    assert_eq!(owners_ref, still_owners_ref);
}

/// Build the fields every audit-row class shares, for the tests below.
fn common(
    audit_id: Uuid,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &str,
    subject_kind: &str,
    written_at: chrono::DateTime<Utc>,
) -> AuditCommon {
    AuditCommon {
        audit_id,
        tenant_id,
        actor_ref,
        action: action.to_owned(),
        subject_kind: subject_kind.to_owned(),
        reason: Some("test reason".to_owned()),
        correlation_id: None,
        written_at,
    }
}

/// Read one `products_audit_log` row by `audit_id`, for the tests below.
async fn find_audit_row(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    audit_id: Uuid,
) -> audit_log::Model {
    audit_log::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(audit_log::Column::AuditId.eq(audit_id)))
        .one(runner)
        .await
        .expect("read the audit row")
        .expect("the audit row exists")
}

/// A refusal row writes and reads back `unsealed`, with all four seam
/// columns `NULL`, its `error_code` present, and the `actor_ref` that was
/// resolved for it.
#[tokio::test]
async fn a_refusal_row_writes_and_reads_back_unsealed_with_its_error_code_and_actor_ref() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("resolve actor ref");

    write_refusal_audit(
        &conn,
        &scope,
        common(
            AUDIT,
            TENANT,
            actor_ref,
            "product.create",
            "product",
            at(10),
        ),
        "DUPLICATE_NAME".to_owned(),
        RefusalSubject::Minted {
            subject_id: PRODUCT,
            subject_revision: Some(1),
        },
    )
    .await
    .expect("write refusal audit row");

    let row = find_audit_row(&conn, &scope, AUDIT).await;

    assert_eq!(row.actor_ref, actor_ref);
    assert_eq!(row.error_code.as_deref(), Some("DUPLICATE_NAME"));
    assert_eq!(row.subject_id, Some(PRODUCT));
    assert_eq!(row.attempted_key, None);
    assert_eq!(
        row.session_id, None,
        "a refusal must never carry a session_id"
    );
    assert_eq!(row.seal_state, "unsealed");
    assert_eq!(row.chain_id, None);
    assert_eq!(row.seq, None);
    assert_eq!(row.prev_hash, None);
    assert_eq!(row.row_hash, None);
}

/// A refusal raised before the mint writes `attempted_key` and leaves
/// `subject_id` `NULL` — the shape a pre-mint refusal takes, since an audit
/// row must never name an id that identifies nothing.
#[tokio::test]
async fn a_pre_mint_refusal_writes_attempted_key_and_leaves_subject_id_null() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("resolve actor ref");

    write_refusal_audit(
        &conn,
        &scope,
        common(
            AUDIT,
            TENANT,
            actor_ref,
            "product.create",
            "product",
            at(10),
        ),
        "DUPLICATE_NAME".to_owned(),
        RefusalSubject::Attempted("Fibre 500".to_owned()),
    )
    .await
    .expect("write refusal audit row");

    let row = find_audit_row(&conn, &scope, AUDIT).await;

    assert_eq!(row.attempted_key.as_deref(), Some("Fibre 500"));
    assert_eq!(row.subject_id, None);
    assert_eq!(row.error_code.as_deref(), Some("DUPLICATE_NAME"));
}

/// An eventless-act row carries neither `error_code` — the refusal class's
/// column — nor `session_id` — the elevated-read class's — since this class
/// is neither.
#[tokio::test]
async fn an_eventless_act_row_carries_neither_error_code_nor_session_id() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(9))
        .await
        .expect("resolve actor ref");

    write_eventless_act_audit(
        &conn,
        &scope,
        common(
            AUDIT,
            TENANT,
            actor_ref,
            "publish.scheduled",
            "product",
            at(10),
        ),
        PRODUCT,
        Some(2),
    )
    .await
    .expect("write eventless act audit row");

    let row = find_audit_row(&conn, &scope, AUDIT).await;

    assert_eq!(row.subject_id, Some(PRODUCT));
    assert_eq!(row.subject_revision, Some(2));
    assert_eq!(row.error_code, None);
    assert_eq!(row.session_id, None);
    assert_eq!(row.seal_state, "unsealed");
}

/// An elevated-read row carries its `session_id` — the break-glass session
/// 05 audits every elevated access with.
#[tokio::test]
async fn an_elevated_read_row_carries_its_session_id() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:auditor", at(9))
        .await
        .expect("resolve actor ref");

    write_elevated_read_audit(
        &conn,
        &scope,
        common(AUDIT, TENANT, actor_ref, "audit.export", "product", at(10)),
        SESSION,
        Some(PRODUCT),
        Some(1),
    )
    .await
    .expect("write elevated read audit row");

    let row = find_audit_row(&conn, &scope, AUDIT).await;

    assert_eq!(row.session_id, Some(SESSION));
    assert_eq!(
        row.error_code, None,
        "an elevated read must never carry an error_code"
    );
    assert_eq!(row.subject_id, Some(PRODUCT));
}

/// A stub door for the flagship test below and its unwritable-audit twin.
///
/// It resolves an `actor_ref` ahead of the gate, in a transaction of its
/// own (`resolve_actor_ref`'s own obligation), attempts to insert a
/// Product, refuses unconditionally, writes the refusal's audit row on a
/// runner of its own — never the mutation's, which has already rolled back
/// by the time this call happens — and finally reports the domain refusal
/// it reached, unless the audit write itself failed, in which case it
/// reports `AUDIT_UNAVAILABLE` instead and never reaches the domain
/// refusal.
///
/// No such door exists in production code: none exists yet in this phase,
/// and Phase 3 owns the real ones. This stub exists only to exercise the
/// two-transaction discipline [`write_refusal_audit`]'s own doc states,
/// which no single-transaction test could otherwise demonstrate.
async fn stub_refusing_door(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    principal_ref: &str,
    now: chrono::DateTime<Utc>,
    product_id: Uuid,
    audit_id: Uuid,
) -> Result<(), DomainError> {
    let resolve_conn = provider
        .conn()
        .expect("scoped connection for actor-ref resolution");
    let actor_ref = resolve_actor_ref(&resolve_conn, scope, tenant_id, principal_ref, now)
        .await
        .expect("resolve actor ref ahead of the gate");

    let scope_for_mutation = scope.clone();
    let new = new_product(product_id, tenant_id);
    let mutation = provider
        .transaction(move |tx| {
            Box::pin(async move {
                insert_product(tx, &scope_for_mutation, new)
                    .await
                    .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                Err::<(), DbError>(DbError::Other(anyhow::Error::msg(
                    "stub door refuses unconditionally",
                )))
            })
        })
        .await;
    assert!(
        mutation.is_err(),
        "the stub door's own mutation must roll back on refusal"
    );

    let audit_conn = provider
        .conn()
        .expect("scoped connection for the refusal audit write");
    write_refusal_audit(
        &audit_conn,
        scope,
        common(
            audit_id,
            tenant_id,
            actor_ref,
            "product.create",
            "product",
            now,
        ),
        "DUPLICATE_NAME".to_owned(),
        RefusalSubject::Minted {
            subject_id: product_id,
            subject_revision: Some(1),
        },
    )
    .await?;

    Err(DomainError::DuplicateName(format!(
        "stub door refusal for product {product_id}"
    )))
}

/// The flagship case: a refused mutation rolls back while its refusal's
/// audit row commits on a separate runner, carrying the `actor_ref` that
/// was resolved for it.
///
/// Asserting only one half would prove nothing about the other — the whole
/// design point of the two-transaction discipline is that the mutation and
/// its audit row have different fates, so both halves are asserted here in
/// one test: the mutation left no row, and the audit row is present naming
/// the same actor.
#[tokio::test]
async fn a_refused_mutation_rolls_back_while_its_refusal_audit_row_commits_separately() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);

    let result = stub_refusing_door(
        &provider,
        &scope,
        TENANT,
        "principal:alice",
        at(9),
        PRODUCT,
        AUDIT,
    )
    .await;
    assert!(
        matches!(result, Err(DomainError::DuplicateName(_))),
        "a successfully audited refusal must still report the domain refusal"
    );

    let conn = provider.conn().expect("scoped connection");

    // Half 1: the refused mutation left no Product row.
    assert_eq!(
        find_product(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read product"),
        None,
        "the refused mutation must have rolled back"
    );

    // Half 2: the refusal's audit row is present, naming the actor the door
    // resolved (the resolution survives the mutation's rollback because it
    // ran, and committed, in its own transaction).
    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:alice", at(20))
        .await
        .expect("resolve the same actor ref again");
    let row = find_audit_row(&conn, &scope, AUDIT).await;
    assert_eq!(row.actor_ref, actor_ref);
    assert_eq!(row.error_code.as_deref(), Some("DUPLICATE_NAME"));
}

/// The unwritable-audit case: the same stub door, but the refusal's audit
/// write is made to fail — here, by seeding a row under the same
/// `audit_id` ahead of the call, so the door's own insert collides on the
/// primary key. The cause of the failure is immaterial; this test is about
/// the door's behaviour when the audit write fails, not about primary-key
/// uniqueness or `chk_products_audit_log_subject_ref` (which this gear's
/// [`RefusalSubject`] cannot even construct a violation of).
///
/// It must answer `AUDIT_UNAVAILABLE` and must not report the domain
/// refusal it had otherwise reached — a refusal the caller learns about and
/// the registry does not is exactly what the "100% write-path audit" NFR
/// forbids.
#[tokio::test]
async fn a_stub_door_with_an_unwritable_refusal_audit_answers_audit_unavailable_not_the_refusal() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);

    let conn = provider.conn().expect("scoped connection");
    let seed_actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:pre-seed", at(1))
        .await
        .expect("resolve a distinct actor ref for the seed row");
    write_refusal_audit(
        &conn,
        &scope,
        common(
            AUDIT,
            TENANT,
            seed_actor_ref,
            "seed.collision",
            "product",
            at(1),
        ),
        "DUPLICATE_NAME".to_owned(),
        RefusalSubject::Attempted("seed".to_owned()),
    )
    .await
    .expect("seed the colliding audit_id");

    let result = stub_refusing_door(
        &provider,
        &scope,
        TENANT,
        "principal:bob",
        at(9),
        PRODUCT,
        AUDIT,
    )
    .await;

    assert!(
        matches!(result, Err(DomainError::AuditUnavailable(_))),
        "an unwritable refusal audit must answer AuditUnavailable, not the domain refusal"
    );
}

/// A read served under elevation that names **no** subject writes its row.
///
/// v1 elevation is audit-export only (`design/01-foundation.md` §4.4), so the
/// ordinary elevated read names no subject at all: it carries a `session_id`
/// and nothing else identifying. `chk_products_audit_log_subject_ref` is this
/// migration's own invention rather than the design set's, and its first
/// draft admitted only a `subject_id` or an `attempted_key` — which refused
/// every subject-less elevated read outright and turned the whole v1
/// elevation class into a permanent `AUDIT_UNAVAILABLE`. The sibling test
/// above passes either way, because it names a subject; only this case can
/// tell the two constraints apart.
#[tokio::test]
async fn an_elevated_read_that_names_no_subject_still_writes_its_audit_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let audit_id = Uuid::from_u128(0xe1e7);
    let session_id = Uuid::from_u128(0x5e55);
    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:auditor-1", at(20))
        .await
        .expect("resolve actor ref");

    write_elevated_read_audit(
        &conn,
        &scope,
        common(
            audit_id,
            TENANT,
            actor_ref,
            "audit.export",
            "audit_log",
            at(20),
        ),
        session_id,
        None,
        None,
    )
    .await
    .expect("a subject-less elevated read must still be recordable");

    let row = find_audit_row(&conn, &scope, audit_id).await;
    assert_eq!(row.session_id, Some(session_id));
    assert_eq!(row.subject_id, None);
    assert_eq!(row.attempted_key, None);
    assert_eq!(row.seal_state, "unsealed");
}

/// A committed eventless act's row shares the mutation's fate: roll the
/// mutation back and the audit row goes with it.
///
/// This is the discipline that distinguishes this class from a refusal's.
/// A refusal's row commits on a runner of its own precisely so it survives
/// the rollback; an eventless act's row commits **inside** the guarded
/// mutation's transaction, so the act and its record stand or fall together.
/// Asserting the row's columns proves nothing about that — only running the
/// write inside a transaction that then fails can tell the two disciplines
/// apart, and without this case a wiring that quietly moved the call onto a
/// separate connection would keep the suite green.
#[tokio::test]
async fn an_eventless_acts_audit_row_rolls_back_with_the_mutation_it_rides_in() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let product_id = Uuid::from_u128(0xac70);
    let audit_id = Uuid::from_u128(0xac71);
    let resolve_conn = provider.conn().expect("scoped connection");
    let actor_ref = resolve_actor_ref(&resolve_conn, &scope, TENANT, "principal:author-9", at(21))
        .await
        .expect("resolve actor ref");

    let scope_for_mutation = scope.clone();
    let new = new_product(product_id, TENANT);
    let mutation = provider
        .transaction(move |tx| {
            Box::pin(async move {
                insert_product(tx, &scope_for_mutation, new)
                    .await
                    .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                write_eventless_act_audit(
                    tx,
                    &scope_for_mutation,
                    common(
                        audit_id,
                        TENANT,
                        actor_ref,
                        "product.schedule",
                        "product",
                        at(21),
                    ),
                    product_id,
                    Some(1),
                )
                .await
                .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                Err::<(), DbError>(DbError::Other(anyhow::Error::msg(
                    "the act fails after its audit row was written",
                )))
            })
        })
        .await;
    assert!(mutation.is_err(), "the mutation must roll back");

    let conn = provider.conn().expect("scoped connection");
    let audit = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(audit_log::Column::AuditId.eq(audit_id)))
        .one(&conn)
        .await
        .expect("read back the audit row");
    assert!(
        audit.is_none(),
        "an eventless act's row commits inside the mutation's transaction, so a rollback must take it too"
    );

    let product = find_product(&conn, &scope, TENANT, product_id)
        .await
        .expect("read back the product");
    assert!(
        product.is_none(),
        "the mutation itself must have rolled back"
    );
}

/// An elevated read whose audit row cannot be written answers
/// `AUDIT_UNAVAILABLE` and serves nothing.
///
/// The discipline is the refusal's, for a different reason: a read has no
/// mutation transaction to join, and an elevated read the registry did not
/// record is exactly what break-glass auditing exists to prevent. The cause
/// of the write failure is immaterial — here a duplicate `audit_id` collides
/// with a row already committed — and this test is about what the caller is
/// handed when the write fails, not about which constraint refused it.
#[tokio::test]
async fn an_elevated_read_with_an_unwritable_audit_answers_audit_unavailable() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let audit_id = Uuid::from_u128(0xe1ee);
    let session_id = Uuid::from_u128(0x5e56);
    let actor_ref = resolve_actor_ref(&conn, &scope, TENANT, "principal:auditor-2", at(22))
        .await
        .expect("resolve actor ref");

    write_elevated_read_audit(
        &conn,
        &scope,
        common(
            audit_id,
            TENANT,
            actor_ref,
            "audit.export",
            "audit_log",
            at(22),
        ),
        session_id,
        None,
        None,
    )
    .await
    .expect("the first elevated read records normally");

    let result = write_elevated_read_audit(
        &conn,
        &scope,
        common(
            audit_id,
            TENANT,
            actor_ref,
            "audit.export",
            "audit_log",
            at(22),
        ),
        session_id,
        None,
        None,
    )
    .await;

    assert!(
        matches!(result, Err(DomainError::AuditUnavailable(_))),
        "an unwritable elevated-read audit must answer AuditUnavailable, and the read must not be served"
    );
}
