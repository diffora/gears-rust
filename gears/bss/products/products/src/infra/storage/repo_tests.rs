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
//!
//! # Only the `SQLite` mirror is executed, and what that leaves unmeasured
//!
//! The whole suite runs in-memory on `SQLite`, so **no case here executes a
//! Postgres statement**. Three things below therefore rest on the
//! Postgres half being the clause-for-clause mirror `migrations_tests.rs`
//! asserts by reading: the head-row guard's `PL/pgSQL` refusal of a
//! `published_version` bump with no frozen row
//! (`a_publish_without_its_frozen_row_is_refused_by_the_head_row_guard`
//! exercises the `SQLite` trigger only), the two partial unique indexes whose
//! `WHERE lifecycle_state <> 'discarded'` predicate is what the discard cases
//! measure as a release, and the `CASE` expression the publish `UPDATE` uses
//! to leave a non-`draft` state alone.
//!
//! The `deprecated` half of "a re-publish takes no edge" **is** measured, by
//! `a_re_publish_from_a_deprecated_head_leaves_it_deprecated`, but its setup
//! is a hand-written `UPDATE` rather than a door: the transition door that
//! writes `published -> deprecated` is not this slice's. The guard judges
//! that setup write exactly as it would judge the door's, so what the case
//! leaves unmeasured is the door, not the rule.
#![allow(clippy::expect_used)]

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, DbBackend, DbErr, EntityTrait, RuntimeErr};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit_db::contention::is_retryable_contention;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    AuditCommon, HeadWrite, IdempotencyAnswer, IdempotencyClaim, NewEntityVersion, NewProduct,
    NewSku, NullableText, ProductHeadSave, RefusalSubject, RepoError, SavedName, SkuHeadSave,
    VersionedEntityKind, answer_idempotency_key, claim_idempotency_key, discard_product_head,
    discard_sku_head, find_non_terminal_skus_of_product, find_product, find_sku,
    insert_entity_version, insert_product, insert_sku, into_product_record, into_sku_record,
    publish_product_head, publish_sku_head, resolve_actor_ref, save_product_head, save_sku_head,
    take_over_expired_idempotency_claim, write_elevated_read_audit, write_eventless_act_audit,
    write_refusal_audit,
};
use crate::domain::error::DomainError;
use crate::infra::storage::entity::{
    audit_log, entity_version, idempotency, identity_ref, product, sku,
};
use crate::infra::storage::migrations::Migrator;
use crate::test_support::at;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const BRAND: Uuid = Uuid::from_u128(0xb1_01);
const PRODUCT: Uuid = Uuid::from_u128(0xf0_01);
const SKU: Uuid = Uuid::from_u128(0x5c_01);
const AUDIT: Uuid = Uuid::from_u128(0xa0_01);
const SESSION: Uuid = Uuid::from_u128(0x5e_01);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);
const APPROVAL: Uuid = Uuid::from_u128(0xa9_01);

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
        cloned_from: None,
        cloned_from_version: None,
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
        cloned_from: None,
        cloned_from_version: None,
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
    // A create writes the column's default and nothing else raises it, so the
    // read-back is the unraised state (P-D-35).
    assert!(!found.composition_pending);
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

/// [`find_non_terminal_skus_of_product`] returns a Product's live children
/// and excludes its terminal ones.
///
/// The exclusion is the half worth a test of its own: it is what keeps a
/// parent's scope narrowing from being refused on account of a `discarded`
/// child nothing can transact against, and it lives in the statement rather
/// than in the caller, so no door-level case would notice if the filter were
/// dropped — every child in a door test is live.
///
/// Two live children rather than one, because the read is documented to
/// order by `sku_code`: a caller refusing on the first offender needs the
/// same offender on every run.
#[tokio::test]
async fn the_child_read_returns_live_skus_and_excludes_terminal_ones() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let second = Uuid::from_u128(0x5c_02);
    let doomed = Uuid::from_u128(0x5c_03);
    let mut live_b = new_sku(SKU, TENANT, PRODUCT);
    live_b.sku_code = "FIBRE-500-B".to_owned();
    let mut live_a = new_sku(second, TENANT, PRODUCT);
    live_a.sku_code = "FIBRE-500-A".to_owned();
    let mut terminal = new_sku(doomed, TENANT, PRODUCT);
    terminal.sku_code = "FIBRE-500-Z".to_owned();
    for new in [live_b, live_a, terminal] {
        insert_sku(&conn, &scope, new).await.expect("insert sku");
    }

    assert_eq!(
        discard_sku_head(&conn, &scope, TENANT, doomed, 1, at(10))
            .await
            .expect("the discard statement runs"),
        HeadWrite::Applied,
        "the third child is put into a terminal state through the repository's own door"
    );

    let children = find_non_terminal_skus_of_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read the children");

    assert_eq!(
        children
            .iter()
            .map(|child| child.sku_code.as_str())
            .collect::<Vec<_>>(),
        vec!["FIBRE-500-A", "FIBRE-500-B"],
        "both live children come back, ordered by sku_code, and the discarded one does not"
    );
}

/// A second Product colliding on `(tenant_id, brand_id, name_normalized)`
/// is refused as [`RepoError::Driver`] — the documented behaviour
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
    assert!(matches!(err, RepoError::Driver { .. }), "got {err:?}");
}

/// A second SKU colliding on `(tenant_id, sku_code)` is refused as
/// [`RepoError::Driver`] — the documented behaviour [`insert_sku`]'s own doc
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
    assert!(matches!(err, RepoError::Driver { .. }), "got {err:?}");
}

/// A SKU inserted against a `product_id` with no matching Product row is
/// refused as [`RepoError::Driver`] — the documented `fk_products_sku_product`
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
    assert!(matches!(err, RepoError::Driver { .. }), "got {err:?}");
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
        cloned_from: None,
        cloned_from_version: None,
        deprecation_provenance: None,
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
        composition_pending: false,
        region_scope: String::new(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        updated_at: at(9),
        cloned_from: None,
        cloned_from_version: None,
        deprecation_provenance: None,
        replaced_by_sku_id: None,
        metering_unit: None,
        usage_type_ref: None,
        correction_ref: None,
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

/// Read one `products_idempotency` row by its composite key, for the tests
/// below.
async fn find_idempotency_row(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
) -> Option<idempotency::Model> {
    idempotency::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(idempotency::Column::TenantId.eq(tenant_id))
                .add(idempotency::Column::Endpoint.eq(endpoint))
                .add(idempotency::Column::ClientKey.eq(client_key)),
        )
        .one(runner)
        .await
        .expect("read idempotency row")
}

/// Transition a claimed idempotency row to `answered` through the
/// repository's own [`answer_idempotency_key`], asserting it reports the row
/// as held.
///
/// An earlier version of this helper wrote the transition by hand, because
/// the repository had no answer-writer at all; it does now, so the setup of
/// every `answered`-state case below runs the same statement production
/// runs. A hand-written setup would let the writer regress while the cases
/// that depend on an `answered` row stayed green.
async fn answer_idempotency_row(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
    response_status: i32,
    response_body: serde_json::Value,
) {
    let outcome = answer_idempotency_key(
        runner,
        scope,
        tenant_id,
        endpoint,
        client_key,
        response_status,
        response_body,
    )
    .await
    .expect("answer the claimed row");
    assert_eq!(
        outcome,
        IdempotencyAnswer::Recorded,
        "this helper's own premise: the key was claimed and is now answered"
    );
}

/// A first claim on a fresh key succeeds and persists a `claimed` row with
/// both response columns `NULL`.
///
/// This is the ordinary path `dod-idempotency-store` exists for: nothing held
/// the key before, so the claim `INSERT` itself is the gate (P-D-42) and
/// there is nothing left to conflict with.
#[tokio::test]
async fn a_first_claim_on_a_fresh_key_succeeds_and_persists_an_unanswered_claimed_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let outcome = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-1",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim a fresh key");

    assert_eq!(outcome, IdempotencyClaim::Claimed);

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-1")
        .await
        .expect("the row exists");
    assert_eq!(row.state, "claimed");
    assert_eq!(row.payload_hash, b"hash-1".to_vec());
    assert_eq!(row.response_status, None);
    assert_eq!(row.response_body, None);
    assert_eq!(row.expires_at, at(11));
}

/// A second claim on a live, unexpired key **carrying the same payload** is
/// refused in flight and writes nothing to the row.
///
/// If this returned `Claimed` a second time, the guarded mutation would run
/// twice under one key — the exact failure the claim `INSERT` being the gate
/// exists to prevent (P-D-42).
///
/// The duplicate deliberately carries the **same** digest as the held claim,
/// because that is what `inst-fd-idem-claim-inflight` reserves the in-flight
/// refusal for: "a duplicate **whose payload hash matches the claimed key's**".
/// An earlier version of this case claimed `hash-1` and retried with
/// `hash-2`, and so proved in-flight for a request the design answers
/// `IDEMPOTENCY_CONFLICT` — the sibling case below is that one, retargeted.
#[tokio::test]
async fn a_second_claim_on_a_live_unexpired_key_answers_in_flight_and_writes_nothing() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-2",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");

    let outcome = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-2",
        b"hash-1",
        at(10),
        at(15),
    )
    .await
    .expect("a duplicate against a live claim does not error");

    assert_eq!(
        outcome,
        IdempotencyClaim::InFlight {
            payload_hash: b"hash-1".to_vec(),
            entity_ref: None,
        },
        "the outcome carries the held digest, which is what lets the door tell this \
         duplicate from one that merely reuses the key"
    );

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-2")
        .await
        .expect("the row exists");
    assert_eq!(
        row.payload_hash,
        b"hash-1".to_vec(),
        "an in-flight refusal must not touch the row it refused against"
    );
    assert_eq!(
        row.expires_at,
        at(11),
        "an in-flight refusal must not touch the row it refused against"
    );
}

/// A second claim on a live, unexpired key **carrying a different payload**
/// reports the **held** digest, not the arriving one, and still writes
/// nothing.
///
/// This is the operand the door's `IDEMPOTENCY_CONFLICT` is computed from,
/// and it has exactly one correct value: the digest already under the key.
/// Returning the caller's own digest would make every comparison agree, and
/// the conflict `inst-fd-idem-conflict` names — "a different payload under a
/// live key fails `IDEMPOTENCY_CONFLICT`, never a silent no-op" — would be
/// structurally unreachable while looking implemented.
///
/// The repository itself still raises nothing here: it was never handed the
/// incoming request to compare, so it reports and the door judges, exactly
/// as for an `answered` row.
#[tokio::test]
async fn a_second_claim_under_a_different_payload_reports_the_held_digest_unchanged() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-2b",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");

    let outcome = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-2b",
        b"hash-2",
        at(10),
        at(15),
    )
    .await
    .expect("a mismatching duplicate is a refusal for the door to classify, not an error");

    assert_eq!(
        outcome,
        IdempotencyClaim::InFlight {
            payload_hash: b"hash-1".to_vec(),
            entity_ref: None,
        },
        "the digest reported is the one the row holds, never the one that just arrived"
    );

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-2b")
        .await
        .expect("the row exists");
    assert_eq!(
        row.payload_hash,
        b"hash-1".to_vec(),
        "a mismatching duplicate owns nothing and overwrites nothing"
    );
    assert_eq!(
        row.expires_at,
        at(11),
        "a mismatching duplicate owns nothing and overwrites nothing"
    );
}

/// A claim against an `answered` row returns the stored response for replay
/// and does not overwrite it.
///
/// The replay is self-contained (P-D-29): the caller can serve
/// `response_status`/`response_body` back without re-executing the guarded
/// mutation or reading anything else, and the stored answer must survive
/// being read this way.
#[tokio::test]
async fn a_claim_against_an_answered_row_returns_the_stored_response_and_does_not_overwrite_it() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-3",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");
    answer_idempotency_row(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-3",
        201,
        json!({"productId": "p-1"}),
    )
    .await;

    let outcome = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-3",
        b"hash-2",
        at(10),
        at(15),
    )
    .await
    .expect("a claim against an answered row does not error");

    assert_eq!(
        outcome,
        IdempotencyClaim::Answered {
            payload_hash: b"hash-1".to_vec(),
            response_status: 201,
            response_body: json!({"productId": "p-1"}),
        }
    );

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-3")
        .await
        .expect("the row exists");
    assert_eq!(
        row.state, "answered",
        "the replay must not overwrite the stored row"
    );
    assert_eq!(row.response_status, Some(201));
    assert_eq!(row.response_body, Some(json!({"productId": "p-1"})));
}

/// A claim against an expired row takes it over: the row's `expires_at`
/// moves to the new deadline and the caller is told it claimed the key.
///
/// Expiry is evaluated at claim time, not by a reaper
/// (`design/01-foundation.md` §3.2, item 3): the very first request past the
/// deadline is what reclaims the key, with no sweep having run. P-D-49 is the
/// decision behind the *compare-and-swap* that makes the takeover safe under
/// contention, which is the sibling case below, not this one.
#[tokio::test]
async fn a_claim_against_an_expired_row_takes_it_over_and_reports_claimed() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-4",
        b"hash-1",
        at(9),
        at(9),
    )
    .await
    .expect("claim with an already-passed expiry");

    let outcome = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-4",
        b"hash-2",
        at(10),
        at(20),
    )
    .await
    .expect("the takeover does not error");

    assert_eq!(outcome, IdempotencyClaim::Claimed);

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-4")
        .await
        .expect("the row exists");
    assert_eq!(row.state, "claimed");
    assert_eq!(row.payload_hash, b"hash-2".to_vec());
    assert_eq!(row.response_status, None);
    assert_eq!(row.response_body, None);
    assert_eq!(row.expires_at, at(20));
}

/// Two claims that both read the same expired row: exactly one wins the
/// takeover and the other is told in flight, having executed nothing.
///
/// This is the case P-D-49 exists for. Nothing holds an expired row between
/// one caller's conflict check and its takeover `UPDATE`, so two duplicates
/// racing on the same expired key both clear the check and both read the
/// same expired row; without the compare-and-swap on `expires_at`, both would
/// be told they claimed it and the guarded mutation would run twice under one
/// key. The interleaving is simulated directly, matching the task's own
/// prescription: both racers' takeover runs against the very same stamp the
/// one read saw, and the loser's `UPDATE` must affect zero rows rather than
/// silently succeed a second time.
///
/// **The loser here carries `hash-b` while the winner wrote `hash-a`, and it
/// is still not a conflict.** That is the one exception to "a payload
/// mismatch stays `IDEMPOTENCY_CONFLICT` in either state": the loser "may
/// even carry a different payload from the winner, and is still refused
/// in-flight rather than for the mismatch, since this transaction never
/// compared the two" (`design/01-foundation.md` §3.2, which cites P-D-49 for
/// the compare-and-swap the sentence rests on; the sentence itself is not in
/// P-D-49's own entry). It read the expired holder's row, not the
/// winner's, so the outcome is `TakeoverRaceLost` — the variant that carries
/// no digest, precisely so no caller can compute a verdict from a hash this
/// transaction never saw.
#[tokio::test]
async fn the_expired_key_takeover_race_admits_exactly_one_winner() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-5",
        b"hash-0",
        at(9),
        at(9),
    )
    .await
    .expect("claim with an already-passed expiry");

    let seen = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-5")
        .await
        .expect("both racers read this same row");

    let winner = take_over_expired_idempotency_claim(&conn, &scope, &seen, b"hash-a", at(20))
        .await
        .expect("the winner's takeover does not error");
    assert_eq!(winner, IdempotencyClaim::Claimed);

    let loser = take_over_expired_idempotency_claim(&conn, &scope, &seen, b"hash-b", at(21))
        .await
        .expect("the loser's takeover does not error either: it is a refusal, not a fault");
    assert_eq!(
        loser,
        IdempotencyClaim::TakeoverRaceLost,
        "the loser's UPDATE must find nothing left matching the stamp it read, and be \
         told in flight rather than told it claimed the key a second time"
    );

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-5")
        .await
        .expect("the row exists");
    assert_eq!(
        row.payload_hash,
        b"hash-a".to_vec(),
        "only the winner's write may be visible"
    );
    assert_eq!(
        row.expires_at,
        at(20),
        "only the winner's write may be visible"
    );
}

/// The key is tenant-scoped: the same `(endpoint, client_key)` in two
/// different tenants both claim successfully.
///
/// If `tenant_id` were not part of the primary key, the second tenant's claim
/// would collide with the first's insert and be refused in flight for a key
/// it never held.
#[tokio::test]
async fn the_same_endpoint_and_client_key_in_two_tenants_both_claim() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let other_scope = AccessScope::for_tenant(OTHER_TENANT);

    let first = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-shared",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("the first tenant claims");
    let second = claim_idempotency_key(
        &conn,
        &other_scope,
        OTHER_TENANT,
        "products/create",
        "key-shared",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("the second tenant claims independently");

    assert_eq!(first, IdempotencyClaim::Claimed);
    assert_eq!(second, IdempotencyClaim::Claimed);
}

/// A claim under a foreign `AccessScope` does not see another tenant's row —
/// the idempotency twin of
/// `resolution_under_a_foreign_scope_does_not_see_another_tenants_ref`.
///
/// A repository that let a foreign scope's insert attempt fall through to
/// another tenant's key would let a caller outside `TENANT` learn, from the
/// refusal it gets back, that the key is already claimed under a tenant it
/// has no access to.
#[tokio::test]
async fn a_claim_under_a_foreign_scope_does_not_see_another_tenants_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let owner_scope = AccessScope::for_tenant(TENANT);
    let foreign_scope = AccessScope::for_tenant(OTHER_TENANT);

    claim_idempotency_key(
        &conn,
        &owner_scope,
        TENANT,
        "products/create",
        "key-7",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("the owner claims normally");

    let err = claim_idempotency_key(
        &conn,
        &foreign_scope,
        TENANT,
        "products/create",
        "key-7",
        b"hash-2",
        at(9),
        at(11),
    )
    .await
    .expect_err("a foreign scope must neither see nor claim under TENANT's row");
    assert!(matches!(err, RepoError::Db(_)));

    // The owner's own claim is untouched by the foreign attempt.
    let row = find_idempotency_row(&conn, &owner_scope, TENANT, "products/create", "key-7")
        .await
        .expect("the row exists");
    assert_eq!(row.payload_hash, b"hash-1".to_vec());
}

/// The answer write moves a claimed row to `answered` and fills **both**
/// response columns in one statement.
///
/// `chk_products_idempotency_response_group` admits `answered` only with the
/// status and the body together, so a writer that set one column, or that
/// moved the state without either, could not commit at all — this case is
/// what proves the single statement carries all three, rather than that the
/// function merely returned `Ok`.
#[tokio::test]
async fn the_answer_write_moves_the_row_to_answered_and_fills_both_response_columns() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-1",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");

    let outcome = answer_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-1",
        201,
        json!({"productId": "p-1"}),
    )
    .await
    .expect("answer the held claim");
    assert_eq!(outcome, IdempotencyAnswer::Recorded);

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-answer-1")
        .await
        .expect("the row exists");
    assert_eq!(row.state, "answered");
    assert_eq!(row.response_status, Some(201));
    assert_eq!(row.response_body, Some(json!({"productId": "p-1"})));
    assert_eq!(
        row.payload_hash,
        b"hash-1".to_vec(),
        "the answer records a response; it must not restamp the digest the claim was made against"
    );
    assert_eq!(
        row.expires_at,
        at(11),
        "the answer records a response; the retention deadline is the claim's own"
    );
}

/// A claim arriving after the answer write reads the stored response back —
/// the two halves of the store joined end to end.
///
/// Each half alone is satisfiable by a broken pairing: a writer that stored
/// the status under the wrong column, or a reader that reported an
/// `answered` row as in flight, is caught only by driving the write and then
/// the read. This is the retry a client actually makes, one layer below the
/// door.
#[tokio::test]
async fn a_claim_after_the_answer_write_replays_the_recorded_response() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-2",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");
    answer_idempotency_row(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-2",
        201,
        json!({"productId": "p-2"}),
    )
    .await;

    let replay = claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-2",
        b"hash-1",
        at(10),
        at(15),
    )
    .await
    .expect("the retry's claim does not error");

    assert_eq!(
        replay,
        IdempotencyClaim::Answered {
            payload_hash: b"hash-1".to_vec(),
            response_status: 201,
            response_body: json!({"productId": "p-2"}),
        },
        "the retry is answered from the row the answer write left behind, not refused in flight"
    );
}

/// An answer write against a row that is not `claimed` reports
/// [`IdempotencyAnswer::NotHeld`] and writes nothing — both when no row
/// exists at all and when one exists already `answered`.
///
/// This is the branch that must never be a silent success. A writer without
/// the `state = 'claimed'` predicate would overwrite the answer already
/// recorded under the second key, replacing one act's outcome with another's
/// — the substitution the store exists to prevent — and a writer that
/// reported zero rows as `Recorded` would let its caller commit an act whose
/// answer was never stored.
#[tokio::test]
async fn an_answer_write_on_a_row_that_is_not_claimed_reports_not_held_and_writes_nothing() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let unclaimed = answer_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-3",
        201,
        json!({"productId": "p-3"}),
    )
    .await
    .expect("answering an unclaimed key is an outcome, not a fault");
    assert_eq!(
        unclaimed,
        IdempotencyAnswer::NotHeld,
        "no row was ever claimed under this key, so there was nothing to answer"
    );
    assert!(
        find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-answer-3")
            .await
            .is_none(),
        "a missed answer must not conjure the row it failed to find"
    );

    claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-4",
        b"hash-1",
        at(9),
        at(11),
    )
    .await
    .expect("claim the key first");
    answer_idempotency_row(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-4",
        201,
        json!({"productId": "p-4"}),
    )
    .await;

    let second = answer_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "products/create",
        "key-answer-4",
        500,
        json!({"productId": "an-act-that-never-ran"}),
    )
    .await
    .expect("a second answer is an outcome, not a fault");
    assert_eq!(
        second,
        IdempotencyAnswer::NotHeld,
        "the row is already answered, so it is not held `claimed` by anyone"
    );

    let row = find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-answer-4")
        .await
        .expect("the row exists");
    assert_eq!(row.response_status, Some(201));
    assert_eq!(
        row.response_body,
        Some(json!({"productId": "p-4"})),
        "the first answer stands: a second write must not replace one act's outcome with another's"
    );
}

/// The answer write commits **inside** the caller's transaction: a rollback
/// takes it, and the key is left free rather than answered.
///
/// `inst-fd-idem-claim-write` requires claim, mutation and answer to commit
/// together or not at all, and only running the write inside a transaction
/// that then fails can tell that wiring from one that answers on a runner of
/// its own. Without this case, a writer that opened its own connection would
/// keep every other case here green while recording, in production, a `201`
/// for an act whose transaction rolled back.
#[tokio::test]
async fn the_answer_write_rolls_back_with_the_transaction_it_rides_in() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let scope_for_mutation = scope.clone();

    let mutation = provider
        .transaction(move |tx| {
            Box::pin(async move {
                claim_idempotency_key(
                    tx,
                    &scope_for_mutation,
                    TENANT,
                    "products/create",
                    "key-answer-5",
                    b"hash-1",
                    at(9),
                    at(11),
                )
                .await
                .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                answer_idempotency_key(
                    tx,
                    &scope_for_mutation,
                    TENANT,
                    "products/create",
                    "key-answer-5",
                    201,
                    json!({"productId": "p-5"}),
                )
                .await
                .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                Err::<(), DbError>(DbError::Other(anyhow::Error::msg(
                    "the act fails after its claim was answered",
                )))
            })
        })
        .await;
    assert!(mutation.is_err(), "the mutation must roll back");

    let conn = provider.conn().expect("scoped connection");
    assert!(
        find_idempotency_row(&conn, &scope, TENANT, "products/create", "key-answer-5")
            .await
            .is_none(),
        "claim and answer commit with the mutation, so a rollback leaves no answered row, and \
         no claimed one either"
    );
}

/// A minimal but well-formed frozen row for `(kind, entity_id)` at `version`.
///
/// The digest bytes and `digest_version` are distinctive rather than zeroed,
/// because the freeze case below asserts the repository stores **what it was
/// handed**: a helper that passed `vec![0; 32]` and `1` would keep a writer
/// that computed its own digest, or defaulted the version, green.
fn frozen_version(kind: VersionedEntityKind, entity_id: Uuid, version: i64) -> NewEntityVersion {
    NewEntityVersion {
        tenant_id: TENANT,
        entity_kind: kind,
        entity_id,
        published_version: version,
        content: r#"{"name":"Fibre 500","productCode":"FIBRE-500"}"#.to_owned(),
        content_digest: (1..=32_u8).collect(),
        digest_version: 7,
        approval_ref: Some(APPROVAL),
        actor_ref: ACTOR,
        published_at: at(10),
    }
}

/// Freeze `version` of `(kind, entity_id)` through the repository's own
/// writer, which is what the head-row guard's existence half looks for.
///
/// Every publish case below goes through this rather than a hand-written
/// insert: a publish probe that seeded the version row by hand would stay
/// green while [`insert_entity_version`] regressed, and the guard would then
/// refuse every real publish with the suite still passing.
async fn freeze(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    kind: VersionedEntityKind,
    entity_id: Uuid,
    version: i64,
) {
    insert_entity_version(runner, scope, frozen_version(kind, entity_id, version))
        .await
        .expect("freeze a version row");
}

/// Read one frozen row back by its whole key.
async fn find_frozen_version(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    kind: &str,
    entity_id: Uuid,
    version: i64,
) -> Option<entity_version::Model> {
    entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(TENANT))
                .add(entity_version::Column::EntityKind.eq(kind))
                .add(entity_version::Column::EntityId.eq(entity_id))
                .add(entity_version::Column::PublishedVersion.eq(version)),
        )
        .one(runner)
        .await
        .expect("read frozen version row")
}

/// The freeze write stores the content, the digest and the digest version it
/// was handed, byte for byte.
///
/// The digest and the rendering are the **door's** to compute; this
/// repository stores bytes. Only a case that hands in bytes it could not have
/// derived — a distinctive digest and a `digest_version` that is not the
/// `1` every other row carries — can tell a writer that stores its input from
/// one that recomputes or defaults it, and slice 10's restore drill compares
/// exactly these bytes.
#[tokio::test]
async fn a_frozen_row_stores_the_content_digest_and_digest_version_it_was_handed() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_entity_version(
        &conn,
        &scope,
        frozen_version(VersionedEntityKind::Product, PRODUCT, 1),
    )
    .await
    .expect("freeze v1");

    let row = find_frozen_version(&conn, &scope, "product", PRODUCT, 1)
        .await
        .expect("the frozen row exists");

    assert_eq!(
        row.content,
        r#"{"name":"Fibre 500","productCode":"FIBRE-500"}"#
    );
    assert_eq!(row.content_digest, (1..=32_u8).collect::<Vec<u8>>());
    assert_eq!(row.digest_version, 7);
    assert_eq!(row.approval_ref, Some(APPROVAL));
    assert_eq!(row.actor_ref, ACTOR);
    assert_eq!(row.published_at, at(10));
}

/// The freeze commits **inside** the caller's transaction: a rollback takes
/// it with the publish it was written for.
///
/// This is the whole reason the function opens no transaction of its own. A
/// freeze that committed separately would leave a version row behind on a
/// rolled-back publish, and the head-row guard would then admit a later
/// `published_version` bump that no committed act produced. Without this case
/// a writer that acquired its own runner would keep every other case here
/// green.
#[tokio::test]
async fn the_freeze_rolls_back_with_the_transaction_it_rides_in() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let scope_for_mutation = scope.clone();

    let mutation = provider
        .transaction(move |tx| {
            Box::pin(async move {
                insert_entity_version(
                    tx,
                    &scope_for_mutation,
                    frozen_version(VersionedEntityKind::Product, PRODUCT, 1),
                )
                .await
                .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                Err::<(), DbError>(DbError::Other(anyhow::Error::msg(
                    "the publish fails after its freeze",
                )))
            })
        })
        .await;
    assert!(mutation.is_err(), "the publish must roll back");

    let conn = provider.conn().expect("scoped connection");
    assert!(
        find_frozen_version(&conn, &scope, "product", PRODUCT, 1)
            .await
            .is_none(),
        "a freeze that survived its rolled-back publish would leave the head-row guard \
         willing to admit a bump no committed act produced"
    );
}

/// Publishing a `draft` Product moves `published_version` and
/// `internal_revision` by exactly one each, in one statement, and writes the
/// `draft -> published` edge.
///
/// Both counters are asserted, not one: the guard bumps `internal_revision`
/// on **every** admitted `UPDATE`, so a two-statement publish would move it
/// twice while `published_version` still read `1`, and a case that checked
/// only the version would pass on it.
#[tokio::test]
async fn publishing_a_draft_product_moves_both_counters_by_exactly_one_and_takes_the_edge() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;

    let outcome = publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("publish the product head");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.published_version, 1, "exactly one version bump");
    assert_eq!(head.internal_revision, 2, "exactly one revision bump");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published
    );
    assert_eq!(head.updated_at, at(10));
}

/// A re-publish of an already `published` Product head moves both counters
/// and leaves `lifecycle_state` alone.
///
/// A re-publish takes no edge. An `UPDATE` that wrote `'published'`
/// unconditionally would be indistinguishable here from one that leaves the
/// state alone, which is why the case that matters is the `deprecated` one
/// this pair's `SKU` twin cannot yet reach: the transition door that produces
/// a `deprecated` head is not this slice's. See the module doc.
#[tokio::test]
async fn a_re_publish_from_a_published_head_leaves_the_state_alone() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;
    publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("first publish");

    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 2).await;
    let outcome = publish_product_head(&conn, &scope, TENANT, PRODUCT, 2, at(11))
        .await
        .expect("re-publish");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.published_version, 2);
    assert_eq!(head.internal_revision, 3);
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published,
        "a re-publish changes the version, never the state"
    );
}

/// A publish carrying a stale expected revision matches no row and is
/// reported, never swallowed.
///
/// The zero-row result is the whole of `STALE_REVISION`'s detection. A
/// function returning `Ok(())` here would tell the door its publish landed
/// while the head still carried the other writer's content.
#[tokio::test]
async fn a_publish_with_a_stale_expected_revision_matches_no_row_and_is_reported() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;

    let outcome = publish_product_head(&conn, &scope, TENANT, PRODUCT, 99, at(10))
        .await
        .expect("the stale publish is an outcome, not a storage failure");
    assert_eq!(outcome, HeadWrite::Unmatched);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.published_version, 0, "nothing was written");
    assert_eq!(head.internal_revision, 1, "nothing was written");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Draft
    );
}

/// A publish under a foreign scope matches no row of the owning tenant.
#[tokio::test]
async fn a_publish_under_a_foreign_scope_does_not_move_another_tenants_head() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let foreign = AccessScope::for_tenant(OTHER_TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;

    let outcome = publish_product_head(&conn, &foreign, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("a foreign publish is an outcome, not a storage failure");
    assert_eq!(outcome, HeadWrite::Unmatched);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.published_version, 0);
    assert_eq!(head.internal_revision, 1);
}

/// Publishing a `draft` SKU moves both counters by exactly one and takes the
/// edge, for the Product case's reasons.
#[tokio::test]
async fn publishing_a_draft_sku_moves_both_counters_by_exactly_one_and_takes_the_edge() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");
    freeze(&conn, &scope, VersionedEntityKind::Sku, SKU, 1).await;

    let outcome = publish_sku_head(&conn, &scope, TENANT, SKU, 1, false, at(10))
        .await
        .expect("publish the sku head");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");
    assert_eq!(head.published_version, 1, "exactly one version bump");
    assert_eq!(head.internal_revision, 2, "exactly one revision bump");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published
    );
    assert_eq!(head.updated_at, at(10));
}

/// A publish whose frozen row was never written is refused by the head-row
/// guard rather than admitted.
///
/// The refusal comes from the database, not from this repository, and it is
/// the reason the freeze must ride the publish's own transaction. Reported as
/// [`RepoError::Driver`]: the guard raises, and the statement fails rather
/// than matching zero rows.
#[tokio::test]
async fn a_publish_without_its_frozen_row_is_refused_by_the_head_row_guard() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let err = publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect_err("the guard refuses a bump with no frozen row");
    assert!(matches!(err, RepoError::Driver { .. }), "got {err:?}");
}

/// Discarding a never-published `draft` Product succeeds and bumps the
/// revision once.
#[tokio::test]
async fn discarding_a_never_published_draft_product_succeeds_and_bumps_the_revision_once() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let outcome = discard_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("discard the draft");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Discarded
    );
    assert_eq!(head.internal_revision, 2);
    assert_eq!(head.published_version, 0, "a discard never publishes");
    assert_eq!(head.updated_at, at(10));
}

/// A discard of a published Product matches zero rows: the legality is in the
/// statement's own filter, so no prior read decides it.
///
/// A read-then-write would race — the head can be published between the read
/// and the write — and the whole point of putting `lifecycle_state = 'draft'`
/// and `published_version = 0` in the `WHERE` clause is that the database
/// judges the row image the write actually lands on.
#[tokio::test]
async fn discarding_a_published_product_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;
    publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("publish");

    let outcome = discard_product_head(&conn, &scope, TENANT, PRODUCT, 2, at(11))
        .await
        .expect("an inadmissible discard is an outcome, not a storage failure");
    assert_eq!(outcome, HeadWrite::Unmatched);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published,
        "nothing was written"
    );
    assert_eq!(head.internal_revision, 2);
}

/// A discarded Product releases both its name and its `product_code`: a
/// second Product takes them.
///
/// Seeded and asserted rather than inferred from the two partial indexes.
/// The release is a property of the discard write, and reading the index
/// definition proves only that the index was declared the way someone
/// intended.
#[tokio::test]
async fn a_discarded_products_name_and_code_are_free_for_a_second_product() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let successor = Uuid::from_u128(0xf0_02);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    let blocked = insert_product(&conn, &scope, new_product(successor, TENANT)).await;
    assert!(
        blocked.is_err(),
        "this case's own premise: the name and the code are held while the first row lives"
    );

    discard_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("discard the draft");

    let taken = insert_product(&conn, &scope, new_product(successor, TENANT))
        .await
        .expect("the discard released the name and the code by its own write");
    assert_eq!(taken.name_normalized, "fibre 500");
    assert_eq!(taken.product_code.as_deref(), Some("FIBRE-500"));
}

/// Discarding a never-published `draft` SKU succeeds and releases its
/// `skuCode`, for the Product case's reasons.
#[tokio::test]
async fn a_discarded_skus_code_is_free_for_a_second_sku() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let successor = Uuid::from_u128(0x5c_02);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");
    let blocked = insert_sku(&conn, &scope, new_sku(successor, TENANT, PRODUCT)).await;
    assert!(
        blocked.is_err(),
        "this case's own premise: the code is reserved while the first row lives"
    );

    let outcome = discard_sku_head(&conn, &scope, TENANT, SKU, 1, at(10))
        .await
        .expect("discard the draft sku");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Discarded
    );
    assert_eq!(head.internal_revision, 2);

    let taken = insert_sku(&conn, &scope, new_sku(successor, TENANT, PRODUCT))
        .await
        .expect("the discard released the reservation by its own write");
    assert_eq!(taken.sku_code, "FIBRE-500-STD");
}

/// A discard of a published SKU matches zero rows, for the Product case's
/// reasons.
#[tokio::test]
async fn discarding_a_published_sku_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");
    freeze(&conn, &scope, VersionedEntityKind::Sku, SKU, 1).await;
    publish_sku_head(&conn, &scope, TENANT, SKU, 1, false, at(10))
        .await
        .expect("publish");

    let outcome = discard_sku_head(&conn, &scope, TENANT, SKU, 2, at(11))
        .await
        .expect("an inadmissible discard is an outcome, not a storage failure");
    assert_eq!(outcome, HeadWrite::Unmatched);

    let head = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Published,
        "nothing was written"
    );
}

/// A discard carrying a stale expected revision matches no row and is
/// reported, for the publish case's reason.
#[tokio::test]
async fn a_discard_with_a_stale_expected_revision_matches_no_row_and_is_reported() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let outcome = discard_product_head(&conn, &scope, TENANT, PRODUCT, 99, at(10))
        .await
        .expect("the stale discard is an outcome, not a storage failure");
    assert_eq!(outcome, HeadWrite::Unmatched);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Draft,
        "nothing was written"
    );
    assert_eq!(head.internal_revision, 1);
}

/// Move a `published` Product head to `deprecated` by the one write the
/// head-row guard admits for it, so the re-publish case below has a head
/// whose state an unconditional write would visibly damage.
///
/// Written here rather than through a repository function because the
/// transition door is not this slice's. It is not a hand-rolled shortcut
/// around the schema: the statement runs against the real table through the
/// same secure wrappers, and the guard's edge clause judges it exactly as it
/// would judge the door's own.
async fn deprecate(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    product_id: Uuid,
    next_internal_revision: i64,
) {
    let moved = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::LifecycleState,
            Expr::value(bss_products_sdk::models::LifecycleState::Deprecated.as_str()),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::value(next_internal_revision),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(at(11)))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(TENANT))
                .add(product::Column::ProductId.eq(product_id)),
        )
        .exec(runner)
        .await
        .expect("the guard admits published -> deprecated");
    assert_eq!(moved.rows_affected, 1, "this helper's own premise");
}

/// A re-publish of a `deprecated` head bumps the version and leaves the head
/// `deprecated`.
///
/// This is the case that tells a `CASE` expression from an unconditional
/// `lifecycle_state = 'published'`: from a `published` head the two writes
/// are indistinguishable, and only a head whose state is neither `draft` nor
/// `published` can show the difference. An unconditional write here would
/// silently undo a deprecation — a state change the transition door owns and
/// a two-person ceremony governs — on a publish that is supposed to change
/// content only.
#[tokio::test]
async fn a_re_publish_from_a_deprecated_head_leaves_it_deprecated() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;
    publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect("first publish");
    deprecate(&conn, &scope, PRODUCT, 3).await;

    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 2).await;
    let outcome = publish_product_head(&conn, &scope, TENANT, PRODUCT, 3, at(12))
        .await
        .expect("re-publish a deprecated head");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.published_version, 2);
    assert_eq!(head.internal_revision, 4);
    assert_eq!(
        head.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Deprecated,
        "a re-publish takes no edge, so the deprecation stands"
    );
}

// ── The retry classifier reads a variant, not a string ────────────────────

/// The two lines the flattening defect consisted of: `DbErr::Custom` is never
/// retryable, and the variant the driver actually raises is.
///
/// This is the whole of the property the publish and discard doors document
/// and, before `RepoError::Driver` existed, did not hold. `RepoError::Db`
/// rendered a `sea_orm::DbErr` into a string at the moment it was raised, and
/// each door re-wrapped that string as `DbErr::Custom`;
/// `is_retryable_contention` matches `DbErr::Exec` and `DbErr::Query` and
/// nothing else, so a genuine `SQLITE_BUSY` collision between two concurrent
/// publishes classified as *not contention* and reached the caller as a bare
/// 500 rather than being re-attempted. Both errors below carry the identical
/// message text: what the classifier reads is the variant, so the text is
/// exactly the thing that cannot carry the signal.
///
/// The `DbErr`s here are hand-built, which is why this is a unit assertion
/// and not a claim about a real collision — see
/// `a_real_driver_failure_is_preserved_as_the_variant_the_driver_raised` for
/// the half that is measured against the database.
#[test]
fn a_stringified_contention_error_is_not_retryable_and_the_preserved_one_is() {
    let text = "error returned from database: (code: 5) database is locked";

    let flattened = RepoError::Db(format!("publish product {PRODUCT}: {text}")).to_db_err();
    assert!(
        matches!(flattened, DbErr::Custom(_)),
        "a string-carrying RepoError has no driver variant left to answer with: {flattened:?}"
    );
    assert!(
        !is_retryable_contention(DbBackend::Sqlite, &flattened),
        "the flattened form is what made a retryable collision a 500"
    );

    let preserved = RepoError::Driver {
        context: format!("publish product {PRODUCT}"),
        source: DbErr::Exec(RuntimeErr::Internal(text.to_owned())),
    }
    .to_db_err();
    assert!(
        matches!(preserved, DbErr::Exec(_)),
        "to_db_err must hand the driver's own variant on unchanged: {preserved:?}"
    );
    assert!(
        is_retryable_contention(DbBackend::Sqlite, &preserved),
        "the preserved form is what lets transaction_with_retry re-attempt the act"
    );
}

/// A failure the `SQLite` driver actually raised arrives as
/// [`RepoError::Driver`] carrying the variant it was raised with, so the
/// classifier's `Exec`/`Query` match is reachable from a real statement and
/// not only from a hand-built error.
///
/// The head-row guard is the one deterministic driver failure this suite can
/// provoke without a second writer: `AFTER UPDATE` on `products_product`
/// raises when a `published_version` bump has no matching frozen row, and the
/// statement fails rather than matching zero rows.
///
/// # What this does not measure, and what it would take
///
/// **It is not a contention probe.** A guard refusal is `Exec`, which is the
/// variant the classifier requires, but its *message* is not a busy or
/// deadlock signature, so `is_retryable_contention` correctly answers `false`
/// for it — asserted below so the case cannot be read as more than it is.
/// The two halves of the fix are therefore measured separately: the variant
/// survives a real statement here, and the classifier's verdict on each
/// variant is pinned by the unit case above.
///
/// A real contention probe needs two writers on **one** database, which this
/// harness cannot supply: `sqlite::memory:` is private to its connection and
/// the pool is pinned to a single connection (`max_conns: Some(1)`), so a
/// second writer would queue on the pool rather than be answered
/// `SQLITE_BUSY` by `SQLite`. It would take a shared database — a
/// `file:...?cache=shared` `DSN` or a temp file — a pool of at least two, and
/// two transactions held open across each other's writes; on Postgres, the
/// `pg` tier plus two connections deliberately deadlocked. Either is a new
/// harness, not a new case in this one.
#[tokio::test]
async fn a_real_driver_failure_is_preserved_as_the_variant_the_driver_raised() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let err = publish_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(10))
        .await
        .expect_err("the guard refuses a bump with no frozen row");

    let db_err = err.to_db_err();
    assert!(
        matches!(db_err, DbErr::Exec(_) | DbErr::Query(_)),
        "the driver's own variant must reach the door, not a rendering of it: {db_err:?}"
    );
    assert!(
        !is_retryable_contention(DbBackend::Sqlite, &db_err),
        "a guard refusal is not contention: the variant is right, the message is not"
    );
}

/// A bucket-iii save writes its columns, bumps the revision once and moves
/// nothing else; the same statement with a **stale** expected revision
/// matches no row and writes nothing.
///
/// The two halves are one case because either alone is passed by a defect the
/// other catches: a statement with no revision filter at all would pass the
/// first, and one whose `col_expr` set never reached the row would pass the
/// second. `HeadWrite::Unmatched` rather than an error is the contract the
/// door reads — it re-reads the head to say *which* refusal it was, and an
/// `Err` would deny it that.
#[tokio::test]
async fn a_bucket_iii_product_save_applies_and_a_stale_one_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let save = ProductHeadSave {
        name: Some(SavedName {
            value: "Fibre 900".to_owned(),
            normalized: "fibre 900".to_owned(),
        }),
        region_scope: Some("eu".to_owned()),
        ..ProductHeadSave::default()
    };
    let outcome = save_product_head(&conn, &scope, TENANT, PRODUCT, 1, &save, at(10))
        .await
        .expect("save the draft head");
    assert_eq!(outcome, HeadWrite::Applied);

    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(head.name, "Fibre 900");
    assert_eq!(
        head.name_normalized, "fibre 900",
        "the index operand moves with the field it is derived from"
    );
    assert_eq!(head.region_scope, "eu");
    assert_eq!(head.internal_revision, 2, "exactly one revision bump");
    assert_eq!(head.published_version, 0, "a save publishes nothing");
    assert_eq!(head.updated_at, at(10));
    assert_eq!(
        head.brand_scope,
        String::new(),
        "a column the save did not name is untouched"
    );

    // The head now carries revision 2; a caller still pinning 1 is stale.
    let stale = save_product_head(&conn, &scope, TENANT, PRODUCT, 1, &save, at(11))
        .await
        .expect("a stale save is an answer, not an error");
    assert_eq!(stale, HeadWrite::Unmatched);
    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        (head.internal_revision, head.updated_at),
        (2, at(10)),
        "the unmatched statement wrote nothing at all"
    );
}

/// A bucket-i save is admitted while `published_version = 0` and matches no
/// row once the head has published — the filter's own
/// `published_version = 0` clause, which rides only where the save names an
/// identity column.
///
/// The bucket-iii save at the end is the control that keeps this from passing
/// against a statement that simply refuses every save on a published head:
/// §4.1 admits a bucket-iii write on any non-terminal head, published or not.
#[tokio::test]
async fn a_bucket_i_product_save_stops_matching_once_the_head_has_published() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let recode = ProductHeadSave {
        product_code: Some(NullableText::Set("FIBRE-900".to_owned())),
        ..ProductHeadSave::default()
    };
    assert_eq!(
        save_product_head(&conn, &scope, TENANT, PRODUCT, 1, &recode, at(10))
            .await
            .expect("save the draft head"),
        HeadWrite::Applied,
        "identity is writable before first publish"
    );

    freeze(&conn, &scope, VersionedEntityKind::Product, PRODUCT, 1).await;
    publish_product_head(&conn, &scope, TENANT, PRODUCT, 2, at(11))
        .await
        .expect("publish the head");

    assert_eq!(
        save_product_head(&conn, &scope, TENANT, PRODUCT, 3, &recode, at(12))
            .await
            .expect("a bucket-i save on a published head is an answer, not an error"),
        HeadWrite::Unmatched,
        "the published_version = 0 clause rides the filter where the save names identity"
    );

    let rename = ProductHeadSave {
        name: Some(SavedName {
            value: "Fibre 900".to_owned(),
            normalized: "fibre 900".to_owned(),
        }),
        ..ProductHeadSave::default()
    };
    assert_eq!(
        save_product_head(&conn, &scope, TENANT, PRODUCT, 3, &rename, at(13))
            .await
            .expect("save the published head"),
        HeadWrite::Applied,
        "the control: a published Product can still be renamed, so the clause is keyed to the \
         bucket and not to the head"
    );
}

/// A SKU save applies, bumps once, and matches no row on a stale revision —
/// [`a_bucket_iii_product_save_applies_and_a_stale_one_matches_no_row`]'s
/// twin over the column set `products_sku` actually has.
#[tokio::test]
async fn a_sku_save_applies_and_a_stale_one_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");

    let save = SkuHeadSave {
        sku_code: Some("FIBRE-900-STD".to_owned()),
        brand_scope: Some("acme".to_owned()),
        ..SkuHeadSave::default()
    };
    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 1, &save, at(10))
            .await
            .expect("save the draft head"),
        HeadWrite::Applied
    );

    let head = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");
    assert_eq!(head.sku_code, "FIBRE-900-STD");
    assert_eq!(head.brand_scope, "acme");
    assert_eq!(
        head.region_scope, "eu",
        "a column the save did not name is untouched"
    );
    assert_eq!(head.internal_revision, 2, "exactly one revision bump");
    assert_eq!(head.published_version, 0);
    assert_eq!(head.updated_at, at(10));

    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 1, &save, at(11))
            .await
            .expect("a stale save is an answer, not an error"),
        HeadWrite::Unmatched
    );
    assert_eq!(
        find_sku(&conn, &scope, TENANT, SKU)
            .await
            .expect("read sku")
            .expect("the row exists")
            .updated_at,
        at(10),
        "the unmatched statement wrote nothing at all"
    );
}

/// A save naming no column at all is a failure rather than a bare revision
/// bump.
///
/// The door refuses an empty payload `VALIDATION` before reaching this, so
/// this is the backstop against a caller this module has not met: a write
/// with no content that still invalidates every `ETag` a client holds.
#[tokio::test]
async fn a_save_naming_no_column_is_refused_rather_than_bumping_the_revision() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let empty = ProductHeadSave::default();
    let failure = save_product_head(&conn, &scope, TENANT, PRODUCT, 1, &empty, at(10))
        .await
        .expect_err("an empty save is refused");
    assert!(
        matches!(failure, RepoError::Db(ref detail) if detail.contains("at least one column")),
        "the internal channel, not a DomainError: a request that reached this would be \
         reporting the gear's own defect, and it was {failure}"
    );
    assert_eq!(
        find_product(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read product")
            .expect("the row exists")
            .internal_revision,
        1,
        "no statement ran"
    );
}

// ── The save statements' own `WHERE` clauses ──────────────────────────────

/// Walk a `draft` Product head to `retired` along the three admitted edges,
/// bumping `internal_revision` on every step and moving `published_version`
/// not at all.
///
/// Written here rather than through a repository function for
/// [`deprecate`]'s reason: no door of this slice retires a head, and these
/// cases need the *state*, not the path. It cannot write `retired` in one
/// statement — `trg_products_product_lifecycle_edge` admits only
/// `draft -> published`, `published -> deprecated` and
/// `deprecated -> retired` — and it cannot skip the bump, which
/// `trg_products_product_internal_revision` requires on every admitted
/// update without exception. Leaving `published_version` at `0` is
/// deliberate: it is what makes the case below measure the **terminal**
/// clause and not the `published_version = 0` one.
async fn retire_product(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    product_id: Uuid,
    revision_after_insert: i64,
) {
    for (step, state) in ["published", "deprecated", "retired"].iter().enumerate() {
        let next =
            revision_after_insert + i64::try_from(step).expect("three steps fit in an i64") + 1;
        let moved = product::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(product::Column::LifecycleState, Expr::value(*state))
            .col_expr(product::Column::InternalRevision, Expr::value(next))
            .col_expr(product::Column::UpdatedAt, Expr::value(at(11)))
            .filter(
                Condition::all()
                    .add(product::Column::TenantId.eq(TENANT))
                    .add(product::Column::ProductId.eq(product_id)),
            )
            .exec(runner)
            .await
            .unwrap_or_else(|e| panic!("the guard admits the edge into `{state}`: {e}"));
        assert_eq!(moved.rows_affected, 1, "this helper's own premise");
    }
}

/// [`retire_product`]'s SKU twin, over the same three edges.
async fn retire_sku(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    sku_id: Uuid,
    revision_after_insert: i64,
) {
    for (step, state) in ["published", "deprecated", "retired"].iter().enumerate() {
        let next =
            revision_after_insert + i64::try_from(step).expect("three steps fit in an i64") + 1;
        let moved = sku::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(sku::Column::LifecycleState, Expr::value(*state))
            .col_expr(sku::Column::InternalRevision, Expr::value(next))
            .col_expr(sku::Column::UpdatedAt, Expr::value(at(11)))
            .filter(
                Condition::all()
                    .add(sku::Column::TenantId.eq(TENANT))
                    .add(sku::Column::SkuId.eq(sku_id)),
            )
            .exec(runner)
            .await
            .unwrap_or_else(|e| panic!("the guard admits the edge into `{state}`: {e}"));
        assert_eq!(moved.rows_affected, 1, "this helper's own premise");
    }
}

/// A save against a `retired` Product head matches no row —
/// [`super::TERMINAL_HEAD_STATES`] in the statement's own filter.
///
/// **This is the clause no door-level case can reach.** The save door asks
/// `transition::check_head_write` first and answers `ENTITY_TERMINAL` from
/// the record it read, so through the router the statement is never even
/// issued against a terminal head. The filter's own copy is the one that
/// decides in the case its doc names: a neighbour retiring the head between
/// the door's read and this write. Only the repository layer can put the
/// statement in that position.
///
/// What the clause's absence would produce is **not** a silent overwrite but
/// something worse to an operator: `trg_products_product_bucket_iii` raises
/// on a bucket-iii write to a terminal head, so the statement would reach the
/// trigger and come back as a driver failure and a 500. The assertion is
/// therefore `HeadWrite::Unmatched` specifically, not merely "the row did not
/// change" — the governed refusal the door turns into a caller-facing answer.
///
/// The admitted save at the top is the positive control: without it the case
/// passes against a filter that matches nothing at all.
#[tokio::test]
async fn a_product_save_against_a_retired_head_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    let rename = |value: &str| ProductHeadSave {
        name: Some(SavedName {
            value: value.to_owned(),
            normalized: value.to_lowercase(),
        }),
        ..ProductHeadSave::default()
    };

    assert_eq!(
        save_product_head(
            &conn,
            &scope,
            TENANT,
            PRODUCT,
            1,
            &rename("Fibre 900"),
            at(10)
        )
        .await
        .expect("save the draft head"),
        HeadWrite::Applied,
        "the positive control: the identical save on a non-terminal head applies"
    );

    retire_product(&conn, &scope, PRODUCT, 2).await;
    let retired = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        retired.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Retired,
        "this case's own premise: the head is terminal"
    );
    assert_eq!(
        retired.published_version, 0,
        "and it got there without ever bumping published_version, so the other clause of \
         this filter is not the one under test"
    );

    assert_eq!(
        save_product_head(
            &conn,
            &scope,
            TENANT,
            PRODUCT,
            5,
            &rename("Fibre 901"),
            at(12)
        )
        .await
        .expect("a terminal save is an answer, not a driver failure"),
        HeadWrite::Unmatched,
        "the terminal clause rides every save's filter, whatever bucket it names"
    );
    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        (head.name.as_str(), head.internal_revision),
        ("Fibre 900", 5),
        "the unmatched statement wrote nothing at all"
    );
}

/// [`a_product_save_against_a_retired_head_matches_no_row`]'s SKU twin, and
/// it is a separate case rather than a loop because the clause is written out
/// twice — once in each save statement — so one of the two could lose it
/// while the other kept it.
///
/// The save deliberately writes a **different** scope from the one the head
/// carries. `trg_products_sku_bucket_iii` fires on
/// `NEW.region_scope IS NOT OLD.region_scope`, so a save re-writing the value
/// already there raises nothing at all: with this clause gone that request
/// would be admitted on a retired head as a bare `internal_revision` bump,
/// caught by no trigger and visible to no other case in this file. The
/// clause is the only thing standing between that request and the row.
#[tokio::test]
async fn a_sku_save_against_a_retired_head_matches_no_row() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");

    let rescope = |value: &str| SkuHeadSave {
        region_scope: Some(value.to_owned()),
        ..SkuHeadSave::default()
    };

    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 1, &rescope("apac"), at(10))
            .await
            .expect("save the draft head"),
        HeadWrite::Applied,
        "the positive control: the identical save on a non-terminal head applies"
    );

    retire_sku(&conn, &scope, SKU, 2).await;
    let retired = find_sku(&conn, &scope, TENANT, SKU)
        .await
        .expect("read sku")
        .expect("the row exists");
    assert_eq!(
        retired.lifecycle_state,
        bss_products_sdk::models::LifecycleState::Retired,
        "this case's own premise: the head is terminal"
    );
    assert_eq!(retired.published_version, 0);

    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 5, &rescope("eu"), at(12))
            .await
            .expect("a terminal save is an answer, not a driver failure"),
        HeadWrite::Unmatched,
        "the terminal clause rides the SKU save's filter too"
    );
    assert_eq!(
        find_sku(&conn, &scope, TENANT, SKU)
            .await
            .expect("read sku")
            .expect("the row exists")
            .region_scope,
        "apac",
        "the unmatched statement wrote nothing at all"
    );
}

/// A bucket-i SKU save is admitted while `published_version = 0` and matches
/// no row once the head has published —
/// [`a_bucket_i_product_save_stops_matching_once_the_head_has_published`]'s
/// twin, which the SKU side did not have.
///
/// The two statements carry the clause separately, so the Product case says
/// nothing about this one. The column exercised is `sku_code`, the identity
/// field `uq_products_sku_code` reserves; the bucket-iii save at the end is
/// the control that keeps the case from passing against a statement that
/// simply refuses every save on a published head, which §4.1 does not.
#[tokio::test]
async fn a_bucket_i_sku_save_stops_matching_once_the_head_has_published() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");
    insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
        .await
        .expect("insert sku");

    let recode = SkuHeadSave {
        sku_code: Some("FIBRE-900-STD".to_owned()),
        ..SkuHeadSave::default()
    };
    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 1, &recode, at(10))
            .await
            .expect("save the draft head"),
        HeadWrite::Applied,
        "identity is writable before first publish"
    );

    freeze(&conn, &scope, VersionedEntityKind::Sku, SKU, 1).await;
    publish_sku_head(&conn, &scope, TENANT, SKU, 2, false, at(11))
        .await
        .expect("publish the head");
    assert_eq!(
        find_sku(&conn, &scope, TENANT, SKU)
            .await
            .expect("read sku")
            .expect("the row exists")
            .published_version,
        1,
        "this case's own premise: the head has published"
    );

    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 3, &recode, at(12))
            .await
            .expect("a bucket-i save on a published head is an answer, not an error"),
        HeadWrite::Unmatched,
        "the published_version = 0 clause rides the filter where the save names identity"
    );

    let rescope = SkuHeadSave {
        region_scope: Some("apac".to_owned()),
        ..SkuHeadSave::default()
    };
    assert_eq!(
        save_sku_head(&conn, &scope, TENANT, SKU, 3, &rescope, at(13))
            .await
            .expect("save the published head"),
        HeadWrite::Applied,
        "the control: a published SKU can still be re-scoped, so the clause is keyed to the \
         bucket and not to the head's state"
    );
}

/// Clearing `product_code` writes `NULL` **and releases the reservation**
/// `uq_products_product_code` was holding.
///
/// [`NullableText::Clear`] exists for exactly this: the index is partial on
/// `product_code IS NOT NULL`, so a cleared code leaves the index by this
/// write and by no other. The stored `NULL` alone is the half that proves
/// little — a write that stored an empty string, or one whose `Expr` bound
/// the literal string `"NULL"`, would be visible, but a `NULL` that somehow
/// failed to leave the index would read back identically. So the case takes
/// the released code with a **second Product**, which can only succeed if the
/// index no longer holds it.
///
/// The refused insert before the clear is the positive control on the
/// reservation itself: without it, a second insert succeeding afterwards
/// would be consistent with the index never having held the code at all.
#[tokio::test]
async fn clearing_a_product_code_writes_null_and_frees_the_code_for_another_product() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
        .await
        .expect("insert product");

    // A distinct name: `uq_products_product_name` would otherwise refuse this
    // row for a reason that has nothing to do with the code under test.
    let contender = Uuid::from_u128(0xf0_02);
    let second = || NewProduct {
        product_id: contender,
        name: "Fibre 700".to_owned(),
        name_normalized: "fibre 700".to_owned(),
        ..new_product(contender, TENANT)
    };

    let refused = insert_product(&conn, &scope, second())
        .await
        .expect_err("the control: while the first Product holds FIBRE-500 the code is taken");
    assert!(
        matches!(refused, RepoError::Driver { .. }),
        "a held reservation surfaces as the driver's own collision, and it was {refused:?}"
    );

    let clear = ProductHeadSave {
        product_code: Some(NullableText::Clear),
        ..ProductHeadSave::default()
    };
    assert_eq!(
        save_product_head(&conn, &scope, TENANT, PRODUCT, 1, &clear, at(10))
            .await
            .expect("clear the code on a draft head"),
        HeadWrite::Applied
    );
    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read product")
        .expect("the row exists");
    assert_eq!(
        head.product_code, None,
        "the clear wrote a real NULL, not an empty string and not the text NULL"
    );
    assert_eq!(head.internal_revision, 2, "one revision bump, as any save");

    insert_product(&conn, &scope, second())
        .await
        .expect("the freed code is takeable, which is the whole point of Clear");
    assert_eq!(
        find_product(&conn, &scope, TENANT, contender)
            .await
            .expect("read the second product")
            .expect("the row exists")
            .product_code
            .as_deref(),
        Some("FIBRE-500"),
        "and the second Product holds it"
    );
}

/// The correction-override evidence and its tripwire (`dod-override-table`).
mod correction_override_tests {
    use sea_orm::{ColumnTrait, Condition, EntityTrait};
    use toolkit_db::secure::{DBRunner, SecureEntityExt};

    use super::super::{
        NewCorrectionOverride, OverrideEvidence, correction_overrides_since,
        record_correction_override,
    };
    use crate::infra::storage::entity::correction_override;

    /// Read one evidence row back through the secure path the repository
    /// itself uses — the only way gear code, test code included, reaches a
    /// row (`DBRunner` is sealed).
    async fn read_override(
        runner: &impl DBRunner,
        scope: &AccessScope,
        override_id: Uuid,
    ) -> correction_override::Model {
        correction_override::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(correction_override::Column::TenantId.eq(TENANT))
                    .add(correction_override::Column::OverrideId.eq(override_id)),
            )
            .one(runner)
            .await
            .expect("the read runs")
            .expect("the row this test just wrote exists")
    }
    use super::*;

    async fn with_a_sku() -> DBProvider<DbError> {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        insert_product(&conn, &scope, new_product(PRODUCT, TENANT))
            .await
            .expect("seed the parent");
        insert_sku(&conn, &scope, new_sku(SKU, TENANT, PRODUCT))
            .await
            .expect("seed the subject");
        db
    }

    fn override_at(at: chrono::DateTime<Utc>) -> NewCorrectionOverride {
        NewCorrectionOverride {
            override_id: Uuid::new_v4(),
            sku_id: SKU,
            field: "sku_code".to_owned(),
            reason: "the ceremony's".to_owned(),
            evidence: OverrideEvidence::UnresolvableTarget {
                target: "sku:missing".to_owned(),
            },
            ceremony_ref: Uuid::from_u128(0xce_01),
            recorded_at: at,
        }
    }

    /// **The tripwire is a windowed count over the rows, so it cannot drift
    /// from them.**
    ///
    /// The `DoD` forbids a counter column or row: *"There is no second piece
    /// of state to drift from the evidence."* This is what that buys — the
    /// count changes because a row exists, and the window is the caller's.
    #[tokio::test]
    async fn the_tripwire_counts_the_window_and_nothing_else() {
        let db = with_a_sku().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let t0 = at(10);

        assert_eq!(
            correction_overrides_since(&conn, &scope, TENANT, t0)
                .await
                .expect("count runs"),
            0,
            "no evidence, no tripwire"
        );

        for offset in [11, 12, 13] {
            record_correction_override(&conn, &scope, TENANT, override_at(at(offset)))
                .await
                .expect("evidence is recorded");
        }

        assert_eq!(
            correction_overrides_since(&conn, &scope, TENANT, t0)
                .await
                .expect("count runs"),
            3
        );
        assert_eq!(
            correction_overrides_since(&conn, &scope, TENANT, at(13))
                .await
                .expect("count runs"),
            1,
            "a narrower window counts fewer rows - the window is the caller's, not stored state"
        );
        assert_eq!(
            correction_overrides_since(&conn, &scope, TENANT, at(23))
                .await
                .expect("count runs"),
            0,
            "a window past every row counts none - `at` takes an hour, so 23 is the late edge"
        );
    }

    /// The arm's evidence round-trips into the column the `CHECK` pins for
    /// it, and the other column stays null.
    ///
    /// **Every column is read back, not just counted.** The `CHECK` already
    /// refuses an arm/column swap, so a count proves only what the DDL
    /// proves; what nothing else guards is a *value* mix-up inside the
    /// admitted shape. Transposing `field` and `reason` in the writer keeps
    /// both `CHECK`s satisfied (both are `<> ''`), keeps the count at two,
    /// and records the corrected field as the justification on the one table
    /// an auditor reads.
    #[tokio::test]
    async fn each_arm_stores_its_own_evidence_column() {
        let db = with_a_sku().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);

        let mut arm_a = override_at(at(11));
        let unavailable_id = arm_a.override_id;
        arm_a.field = "metering_unit".to_owned();
        arm_a.reason = "the collector deleted the unit".to_owned();
        arm_a.evidence = OverrideEvidence::ProducerUnavailable {
            snapshot: "{\"pricing\":\"stale\"}".to_owned(),
        };
        let ceremony_a = arm_a.ceremony_ref;
        record_correction_override(&conn, &scope, TENANT, arm_a)
            .await
            .expect("arm (a) is admitted");

        let mut arm_b = override_at(at(12));
        let unresolvable_id = arm_b.override_id;
        arm_b.field = "usage_type_ref".to_owned();
        arm_b.reason = "the target no longer resolves".to_owned();
        record_correction_override(&conn, &scope, TENANT, arm_b)
            .await
            .expect("arm (b) is admitted");

        assert_eq!(
            correction_overrides_since(&conn, &scope, TENANT, at(10))
                .await
                .expect("count runs"),
            2,
            "both arms are evidence, and both count"
        );

        let row_a = read_override(&conn, &scope, unavailable_id).await;
        assert_eq!(row_a.sku_id, SKU);
        assert_eq!(
            row_a.field, "metering_unit",
            "the corrected field, not the reason"
        );
        assert_eq!(
            row_a.reason, "the collector deleted the unit",
            "the justification, not the field"
        );
        assert_eq!(row_a.admitting_arm, "producer_unavailable");
        assert_eq!(
            row_a.unavailability_snapshot.as_deref(),
            Some("{\"pricing\":\"stale\"}"),
            "arm (a)'s evidence lands in arm (a)'s column"
        );
        assert_eq!(
            row_a.unresolvable_target, None,
            "and the other column stays null"
        );
        assert_eq!(
            row_a.ceremony_ref, ceremony_a,
            "the ceremony reference is the caller's"
        );

        let row_b = read_override(&conn, &scope, unresolvable_id).await;
        assert_eq!(row_b.field, "usage_type_ref");
        assert_eq!(row_b.reason, "the target no longer resolves");
        assert_eq!(row_b.admitting_arm, "unresolvable_target");
        assert_eq!(
            row_b.unresolvable_target.as_deref(),
            Some("sku:missing"),
            "arm (b)'s evidence lands in arm (b)'s column"
        );
        assert_eq!(row_b.unavailability_snapshot, None);
    }
}

/// The approval store — the submit write, the decision write and the queue
/// read (`dod-stored-snapshot`, `dod-self-approval`).
mod approval_store_tests {
    use super::super::{
        ApprovalStoreError, DecisionVerdict, NewApproval, NewDecision, pending_approvals,
        read_approval, record_decision, submit_approval,
    };
    use super::*;
    use crate::domain::approval::{describe_quorum, descriptor_from_stored};
    use crate::domain::governance::{ApprovalId, EntityRef, GateSubject};
    use crate::domain::materiality::{
        MaterialAct, Materiality, MaterialityEvaluator, MaterialityPolicy, Resolution,
    };

    /// A resolved claim set — the evaluator refuses without one, so every
    /// probe below carries it.
    fn claim_set() -> Vec<String> {
        vec!["catalog-admin".to_owned()]
    }

    const AUTHOR: Uuid = Uuid::from_u128(0x9001);
    const APPROVER: Uuid = Uuid::from_u128(0x9002);

    fn subject() -> GateSubject {
        GateSubject::entity_publish(EntityRef {
            tenant_id: TENANT,
            entity_kind: bss_products_sdk::models::EntityKind::Product,
            entity_id: PRODUCT,
        })
    }

    /// A material act, so every probe below judges the same way and the
    /// count in the descriptor comes from `approver_count` alone.
    const MATERIAL: MaterialAct<'static> = MaterialAct::PolicyMutation;

    fn submission<'a>(
        id: ApprovalId,
        subject: &'a GateSubject,
        content: &'a str,
        basis: Option<i64>,
        evaluator: MaterialityEvaluator<'a>,
        approver_count: u32,
    ) -> NewApproval<'a> {
        NewApproval {
            approval_id: id,
            subject,
            internal_revision: 1,
            content_snapshot: content,
            diff_basis: basis,
            act: &MATERIAL,
            evaluator,
            finance_material: false,
            approver_count,
            submitter: AUTHOR,
            author_override_ack: None,
        }
    }

    /// **The submitted content and the descriptor are both stored**, and the
    /// stored descriptor round-trips into the same value — so a reader takes
    /// the record's own count rather than recomputing one.
    #[tokio::test]
    async fn a_submission_stores_its_snapshot_and_its_descriptor() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
        let id = ApprovalId::new(Uuid::new_v4());

        let answered = submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"as submitted"}"#, Some(7), ev, 2),
            at(10),
        )
        .await
        .expect("the submission is stored");
        assert_eq!(
            answered.materiality,
            Materiality::Material,
            "the store evaluated the act itself, at submission"
        );

        let row = read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the row exists");
        assert_eq!(row.content_snapshot, r#"{"name":"as submitted"}"#);
        assert_eq!(row.diff_basis, Some(7));
        assert_eq!(row.state, "pending");
        assert_eq!(row.submitter, AUTHOR);
        assert_eq!(row.author_override_ack, None);
        assert_eq!(row.author_override_ack_at, None);
        assert_eq!(
            descriptor_from_stored(&row.quorum_descriptor).expect("the stored descriptor decodes"),
            answered.descriptor,
            "the descriptor round-trips: a reader never recomputes N"
        );
        assert_eq!(
            answered.descriptor,
            describe_quorum(Materiality::Material, 2, false),
            "and it is the descriptor the N in force produces"
        );
    }

    /// A first publish stores a NULL basis, which is the arm the `DoD` states
    /// because filling it by convention would diff the draft against the
    /// head.
    #[tokio::test]
    async fn a_first_publish_stores_a_null_basis() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"first"}"#, None, ev, 2),
            at(10),
        )
        .await
        .expect("the submission is stored");
        assert_eq!(
            read_approval(&conn, &scope, TENANT, id)
                .await
                .expect("read runs")
                .expect("the row exists")
                .diff_basis,
            None,
            "a real NULL, not a zero"
        );
    }

    /// **The flagship probe, at the store**: submit, edit the head, submit
    /// again — the superseded record still carries the ORIGINAL bytes.
    ///
    /// The head is not touched at all here, and that is the point: nothing
    /// in the read path can reach it, so the first record's snapshot cannot
    /// have been re-derived.
    #[tokio::test]
    async fn a_superseded_record_keeps_the_content_it_was_submitted_with() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let first = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(
                first,
                &subject,
                r#"{"name":"as submitted"}"#,
                Some(7),
                ev,
                2,
            ),
            at(10),
        )
        .await
        .expect("the first submission is stored");

        // The author edits and re-submits. L-4: the new submission
        // supersedes the open one rather than being refused.
        let second = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(second, &subject, r#"{"name":"edited"}"#, Some(7), ev, 2),
            at(11),
        )
        .await
        .expect("the second submission supersedes rather than colliding");

        let superseded = read_approval(&conn, &scope, TENANT, first)
            .await
            .expect("read runs")
            .expect("the row exists");
        assert_eq!(
            superseded.state, "superseded",
            "L-4: a new submission explicitly supersedes the open one"
        );
        assert_eq!(
            superseded.content_snapshot, r#"{"name":"as submitted"}"#,
            "the superseded record renders the ORIGINAL submission, never the edit"
        );
        assert_eq!(
            read_approval(&conn, &scope, TENANT, second)
                .await
                .expect("read runs")
                .expect("the row exists")
                .content_snapshot,
            r#"{"name":"edited"}"#
        );
    }

    /// **The author's own decision is refused at the store**, by principal.
    #[tokio::test]
    async fn the_author_cannot_decide_their_own_record() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("the submission is stored");

        let err = record_decision(
            &conn,
            &scope,
            NewDecision {
                tenant_id: TENANT,
                approval_id: id,
                approver_principal: AUTHOR,
                verdict: DecisionVerdict::Approved,
                reason: None,
                override_acknowledgments: None,
            },
            AUTHOR,
            at(11),
        )
        .await
        .expect_err("an author may never decide their own record");
        assert!(
            err.to_string().contains("SELF_APPROVAL_FORBIDDEN"),
            "the refusal carries its code: {err}"
        );

        // The paired positive control: a different principal is admitted on
        // the same record, so the refusal is the principal's and not the
        // record's.
        record_decision(
            &conn,
            &scope,
            NewDecision {
                tenant_id: TENANT,
                approval_id: id,
                approver_principal: APPROVER,
                verdict: DecisionVerdict::Approved,
                reason: Some("looks right"),
                override_acknowledgments: None,
            },
            APPROVER,
            at(11),
        )
        .await
        .expect("a distinct principal is admitted");
    }

    /// **One principal, one decision** — C2's UNIQUE, read back as the
    /// refusal it is rather than as a supersession.
    #[tokio::test]
    async fn one_principal_decides_once_whatever_roles_they_hold() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("the submission is stored");

        // A rejection carries a mandatory reason (design/05 §2), so the
        // second call supplies one — otherwise it would be refused for that
        // and the UNIQUE would go unprobed.
        let decision = |verdict, reason| NewDecision {
            tenant_id: TENANT,
            approval_id: id,
            approver_principal: APPROVER,
            verdict,
            reason,
            override_acknowledgments: None,
        };
        record_decision(
            &conn,
            &scope,
            decision(DecisionVerdict::Approved, None),
            APPROVER,
            at(11),
        )
        .await
        .expect("the first verdict is cast");
        let err = record_decision(
            &conn,
            &scope,
            decision(DecisionVerdict::Rejected, Some("on reflection")),
            APPROVER,
            at(12),
        )
        .await
        .expect_err("a second verdict from the same principal is refused");
        assert!(
            err.to_string().contains("one principal, one decision"),
            "the refusal names C2's rule: {err}"
        );
    }

    /// The author's acknowledgment is admitted **only** at effective quorum
    /// zero, and refused above it — so it cannot become a second,
    /// unpoliced acknowledgment channel.
    #[tokio::test]
    async fn the_author_acknowledgment_is_admitted_only_at_quorum_zero() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
        let mut supplied = submission(
            ApprovalId::new(Uuid::new_v4()),
            &subject,
            r#"{"name":"x"}"#,
            Some(1),
            ev,
            2,
        );
        supplied.author_override_ack = Some("lint:name-collision");
        let err = submit_approval(&conn, &scope, supplied, at(10))
            .await
            .expect_err("above zero the acknowledgment rides the decision row");
        assert!(
            err.to_string().contains("effective quorum zero"),
            "the refusal names the one admitted home: {err}"
        );

        let id = ApprovalId::new(Uuid::new_v4());
        let mut at_zero = submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 0);
        at_zero.author_override_ack = Some("lint:name-collision");
        submit_approval(&conn, &scope, at_zero, at(10))
            .await
            .expect("at N = 0 the author acknowledges on the record");
        let row = read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the row exists");
        assert_eq!(
            row.author_override_ack.as_deref(),
            Some("lint:name-collision")
        );
        assert!(
            row.author_override_ack_at.is_some(),
            "the CHECK pins the pair: both columns or neither"
        );
    }

    /// **A decision on a record that is no longer open is refused**
    /// `APPROVAL_SUPERSEDED` (409). `products_approval_decision` is
    /// append-only outright, so a verdict cast on a closed ceremony cannot
    /// be taken back and any evaluator counting principals by `approval_id`
    /// would count it.
    #[tokio::test]
    async fn a_decision_on_a_superseded_record_is_refused() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let first = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(first, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("stored");
        // The author re-submits: the first record is superseded.
        submit_approval(
            &conn,
            &scope,
            submission(
                ApprovalId::new(Uuid::new_v4()),
                &subject,
                r#"{"name":"y"}"#,
                Some(1),
                ev,
                2,
            ),
            at(11),
        )
        .await
        .expect("stored");

        let err = record_decision(
            &conn,
            &scope,
            NewDecision {
                tenant_id: TENANT,
                approval_id: first,
                approver_principal: APPROVER,
                verdict: DecisionVerdict::Approved,
                reason: None,
                override_acknowledgments: None,
            },
            APPROVER,
            at(12),
        )
        .await
        .expect_err("a closed ceremony admits no verdict");
        match err {
            ApprovalStoreError::Refused(d) => assert_eq!(d.code(), "APPROVAL_SUPERSEDED"),
            other @ ApprovalStoreError::Repo(_) => {
                panic!("expected the declared 409, got {other}")
            }
        }
    }

    /// **A record that closes on no approver admits no decision row**
    /// (P-D-68 arm 1) — the reason the author's acknowledgment gets a
    /// column. `decision_admitted`'s `>= 1` guard is silent at zero by
    /// construction, so this refusal is separate.
    #[tokio::test]
    async fn a_record_closing_on_no_approver_admits_no_decision() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 0),
            at(10),
        )
        .await
        .expect("stored at N = 0");
        for principal in [AUTHOR, APPROVER] {
            let err = record_decision(
                &conn,
                &scope,
                NewDecision {
                    tenant_id: TENANT,
                    approval_id: id,
                    approver_principal: principal,
                    verdict: DecisionVerdict::Approved,
                    reason: None,
                    override_acknowledgments: None,
                },
                principal,
                at(11),
            )
            .await
            .expect_err("at required = 0 no principal may decide, the author least of all");
            assert!(err.to_string().contains("closes on no approver"), "{err}");
        }
    }

    /// **A verdict may not be attributed to another principal.** Without
    /// this an author holding `approval x decide` names a second principal
    /// and satisfies C2's two-person invariant alone — on an append-only row.
    #[tokio::test]
    async fn a_verdict_cannot_be_attributed_to_someone_else() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("stored");
        let err = record_decision(
            &conn,
            &scope,
            NewDecision {
                tenant_id: TENANT,
                approval_id: id,
                approver_principal: APPROVER,
                verdict: DecisionVerdict::Approved,
                reason: None,
                override_acknowledgments: None,
            },
            AUTHOR,
            at(11),
        )
        .await
        .expect_err("the acting principal must be the one the row names");
        assert!(err.to_string().contains("may not cast a verdict"), "{err}");
    }

    /// **A rejection carries a mandatory reason** (`design/05` §2). No
    /// `CHECK` constrains the column, so the rule lives at the store or
    /// nowhere — and an unreasoned rejection leaves the record's only
    /// evidence silent about why.
    #[tokio::test]
    async fn an_unreasoned_rejection_is_refused() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let id = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("stored");
        let reject = |reason| NewDecision {
            tenant_id: TENANT,
            approval_id: id,
            approver_principal: APPROVER,
            verdict: DecisionVerdict::Rejected,
            reason,
            override_acknowledgments: None,
        };
        let err = record_decision(&conn, &scope, reject(None), APPROVER, at(11))
            .await
            .expect_err("a rejection with no reason is refused");
        assert!(err.to_string().contains("mandatory reason"), "{err}");
        // The paired positive control: with a reason it lands, so the
        // refusal above is the reason's absence and not the verdict.
        record_decision(
            &conn,
            &scope,
            reject(Some("the tier is wrong")),
            APPROVER,
            at(11),
        )
        .await
        .expect("a reasoned rejection is admitted");
    }

    /// **The store evaluates the act it was handed**, and a non-material one
    /// produces a different stored descriptor. Every other probe here
    /// submits one act shape, so without this a `submit_approval` that
    /// hardcoded `Materiality::Material` and dropped `finance_material`
    /// would be green.
    #[tokio::test]
    async fn the_store_evaluates_the_act_and_reads_the_finance_flag() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        // A batch act below the trigger is NON-material, so `required`
        // becomes `min(N, 1) = 1` at `N = 3` rather than 3.
        let small_batch = MaterialAct::BatchAct { affected: 1 };
        let id = ApprovalId::new(Uuid::new_v4());
        let mut sub = submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 3);
        sub.act = &small_batch;
        let answered = submit_approval(&conn, &scope, sub, at(10))
            .await
            .expect("stored");
        assert_eq!(
            answered.materiality,
            Materiality::NonMaterial,
            "the store judged the act it was handed, not a constant"
        );
        assert_eq!(answered.descriptor.required(), 1);
        assert_eq!(answered.descriptor.configured_quorum(), 3);

        // And the finance flag reaches the descriptor.
        let id = ApprovalId::new(Uuid::new_v4());
        let mut sub = submission(id, &subject, r#"{"name":"y"}"#, Some(1), ev, 2);
        sub.finance_material = true;
        let answered = submit_approval(&conn, &scope, sub, at(11))
            .await
            .expect("stored");
        assert!(
            answered.descriptor.finance_required(),
            "finance_material is read, not dropped"
        );
    }

    /// **An unresolvable input refuses the submission**, so nothing is
    /// stored — the fail-closed clause, at the store rather than only in the
    /// domain module.
    #[tokio::test]
    async fn an_unresolvable_input_stores_nothing() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let subject = subject();
        let claims = claim_set();
        let ev = MaterialityEvaluator::new(Resolution::Unresolvable, Resolution::Resolved(&claims));
        let id = ApprovalId::new(Uuid::new_v4());
        let err = submit_approval(
            &conn,
            &scope,
            submission(id, &subject, r#"{"name":"x"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect_err("an absent policy refuses the act");
        assert!(err.to_string().contains("materiality_policy"), "{err}");
        assert!(
            read_approval(&conn, &scope, TENANT, id)
                .await
                .expect("read runs")
                .is_none(),
            "the refusal stored nothing"
        );
    }

    /// The queue read returns pending records **oldest first**, and a
    /// superseded record leaves it — `inst-gv-queue`'s operand.
    ///
    /// **Two subjects, submitted out of order.** One subject holds one open
    /// record, so a one-subject fixture leaves the queue one element long
    /// and the ordering half unprobed: `Order::Desc`, or no `order_by` at
    /// all, would pass. Two subjects also make the *filter* visible — a
    /// superseded row leaves the queue because the read filters
    /// `state = 'pending'`, not because of the partial UNIQUE, which
    /// constrains writes and removes nothing from a read.
    #[tokio::test]
    async fn the_pending_queue_is_oldest_first_and_carries_only_pending() {
        let db = harness().await;
        let conn = db.conn().expect("conn");
        let scope = AccessScope::for_tenant(TENANT);
        let policy = MaterialityPolicy::default();
        let claims = claim_set();
        let ev =
            MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

        let product = subject();
        let sku = GateSubject::entity_publish(EntityRef {
            tenant_id: TENANT,
            entity_kind: bss_products_sdk::models::EntityKind::Sku,
            entity_id: SKU,
        });

        // The LATER submission goes in FIRST, so an unordered read cannot
        // pass by insertion accident.
        let newer = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(newer, &sku, r#"{"name":"newer"}"#, Some(1), ev, 2),
            at(14),
        )
        .await
        .expect("stored");
        let older = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(older, &product, r#"{"name":"older"}"#, Some(1), ev, 2),
            at(10),
        )
        .await
        .expect("stored");

        let queue = pending_approvals(&conn, &scope, TENANT)
            .await
            .expect("the queue reads");
        assert_eq!(queue.len(), 2, "two subjects, two open records");
        assert_eq!(queue[0].approval_id, older.get(), "oldest first");
        assert_eq!(queue[1].approval_id, newer.get());

        // Re-submitting on the older subject supersedes its record, and the
        // superseded row leaves the queue.
        let replacing = ApprovalId::new(Uuid::new_v4());
        submit_approval(
            &conn,
            &scope,
            submission(replacing, &product, r#"{"name":"re"}"#, Some(1), ev, 2),
            at(15),
        )
        .await
        .expect("a re-submission supersedes rather than colliding");
        let queue = pending_approvals(&conn, &scope, TENANT)
            .await
            .expect("the queue reads");
        assert_eq!(queue.len(), 2, "still one open record per subject");
        assert!(
            queue.iter().all(|r| r.approval_id != older.get()),
            "the superseded record left the queue: the read filters state = 'pending'"
        );
    }
}
