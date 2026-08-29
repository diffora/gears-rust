//! `products_product` / `products_sku` repository tests, against the executed
//! `SQLite` mirror.
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
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    NewProduct, NewSku, RepoError, find_product, find_sku, insert_product, insert_sku,
    into_product_record, into_sku_record,
};
use crate::infra::storage::entity::{product, sku};
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const BRAND: Uuid = Uuid::from_u128(0xb1_01);
const PRODUCT: Uuid = Uuid::from_u128(0xf0_01);
const SKU: Uuid = Uuid::from_u128(0x5c_01);

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
