//! Persistence probes for the staleness stamp host — every obligation in
//! `dod-staleness-stamp` that a domain-only test cannot arm against the
//! table.
#![allow(clippy::expect_used)]

use chrono::{TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    NewReadEntity, apply_read_stamp, count_read_entities, delete_read_entity, insert_read_entity,
    load_read_stamp, scope_condition, visibility_condition,
};
use crate::domain::read_model::{
    ReadSurface, StampApply, StampCatalogTouch, VisibilityFilter, completeness_rejects_removal,
    floor_admits_removal,
};
use crate::infra::storage::entity::read_entity;
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENTITY: Uuid = Uuid::from_u128(0xdd_11);
const VERSION: i64 = 0xee_11;

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
        .expect("boot the migration chain");
    DBProvider::<DbError>::new(db)
}

/// A zero-version tenant's first apply stamps `null` with a real
/// `projectedAt`, and a load reads both halves back — absence of the
/// version field would be indistinguishable from a dropped stamp.
#[tokio::test]
async fn a_zero_version_tenant_persists_null_with_a_projected_at() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();

    let written = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Anchorless,
            projected_at: at,
            entities_projected: true,
        },
    )
    .await
    .expect("bootstrap apply");
    assert_eq!(written.as_of_catalog_version, None);
    assert_eq!(written.projected_at, at);

    let loaded = load_read_stamp(&conn, &scope, TENANT)
        .await
        .expect("load")
        .expect("the bootstrap left a row");
    assert_eq!(loaded.as_of_catalog_version, None);
    assert_eq!(loaded.projected_at, at);
}

/// Ordering against the table: stamping a catalog version before the
/// changed-entity list is projected is refused, and no row is written.
#[tokio::test]
async fn a_premature_catalog_stamp_writes_nothing() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();

    let err = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Set(VERSION),
            projected_at: at,
            entities_projected: false,
        },
    )
    .await
    .expect_err("must refuse before entities are projected");
    assert!(
        err.to_string().contains("entities not yet projected"),
        "got {err}"
    );
    assert!(
        load_read_stamp(&conn, &scope, TENANT)
            .await
            .expect("load")
            .is_none(),
        "a refused advance must leave no stamp row"
    );
}

/// Floor vs completeness, armed on a **removal** that hits the tables: a
/// retirement deletes a serving row, the stamp's catalog version stays, and
/// `projected_at` advances. Completeness would alarm; the floor admits it.
#[tokio::test]
async fn a_retirement_removal_advances_projected_at_without_moving_the_version() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let t0 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 1).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 2).unwrap();

    apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Set(VERSION),
            projected_at: t0,
            entities_projected: true,
        },
    )
    .await
    .expect("seed the stamp at a catalog version");
    insert_read_entity(
        &conn,
        &scope,
        NewReadEntity {
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            name: "Fibre 500".to_owned(),
            lifecycle_state: "published".to_owned(),
            published_version: 1,
            projected_at: t0,
        },
    )
    .await
    .expect("seed one serving row");
    assert_eq!(count_read_entities(&conn, &scope, TENANT).await.unwrap(), 1);

    // The retirement flip: content goes, catalog version does not.
    assert_eq!(
        delete_read_entity(&conn, &scope, TENANT, "sku", ENTITY)
            .await
            .expect("remove"),
        1
    );
    let before = load_read_stamp(&conn, &scope, TENANT)
        .await
        .expect("load")
        .expect("stamp");
    let after = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Unchanged,
            projected_at: t1,
            entities_projected: true,
        },
    )
    .await
    .expect("the retirement apply still stamps");

    let rows_after = count_read_entities(&conn, &scope, TENANT).await.unwrap();
    assert_eq!(rows_after, 0);
    assert_eq!(after.as_of_catalog_version, Some(VERSION));
    assert_eq!(after.projected_at, t1);
    assert!(completeness_rejects_removal(1, rows_after, true));
    assert!(floor_admits_removal(1, rows_after, &before, &after));

    // And a later apply still advances projected_at with the version held.
    let later = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Unchanged,
            projected_at: t2,
            entities_projected: true,
        },
    )
    .await
    .expect("version-or-none apply");
    assert_eq!(later.as_of_catalog_version, Some(VERSION));
    assert_eq!(later.projected_at, t2);
}

// -- The two query-build predicates, moved here with the functions (P-D-163) --

/// **The contract renders as an `IN` over the served states**, so a row a
/// caller may not see is never fetched. The rendering is asserted rather
/// than the shape, because "applied at query build" is a claim about the SQL
/// and not about where the code sits.
#[test]
fn the_filter_renders_an_in_over_served_states_not_a_negation() {
    use sea_orm::{EntityTrait, QueryFilter, QueryTrait};
    let sql = read_entity::Entity::find()
        .filter(visibility_condition(VisibilityFilter::for_surface(
            ReadSurface::DefaultBrowse,
        )))
        .build(sea_orm::DatabaseBackend::Sqlite)
        .to_string();
    assert!(sql.contains("IN ("), "the predicate is an IN: {sql}");
    assert!(
        !sql.contains("NOT IN"),
        "a NOT IN over withheld states would serve any state added later: {sql}"
    );
    assert!(
        sql.contains("'published'") && sql.contains("'deprecated'"),
        "{sql}"
    );
    for withheld in ["'retired'", "'draft'", "'discarded'"] {
        assert!(
            !sql.contains(withheld),
            "{withheld} reached the query: {sql}"
        );
    }
}

/// The rendered statement for one claim: its SQL and its bound values.
///
/// **The values, not the SQL text.** `Expr::cust_with_exprs` binds its
/// operands as parameters, so the pattern never appears in the statement
/// string — and that difference is exactly what tells a substring match from
/// a token match, which is why the first probe here could not see the leak.
fn scope_sql(claim: &str) -> (String, Vec<String>) {
    use sea_orm::{EntityTrait, QueryFilter, QueryTrait};
    let stmt = read_entity::Entity::find()
        .filter(scope_condition(read_entity::Column::RegionScope, claim))
        .build(sea_orm::DatabaseBackend::Sqlite);
    let values = stmt
        .values
        .as_ref()
        .map(|v| v.0.iter().map(std::string::ToString::to_string).collect())
        .unwrap_or_default();
    (stmt.to_string(), values)
}

/// **The scope predicate admits the unrestricted row** (P-D-39). Containment
/// alone hides the whole catalogue of a tenant that has set no scopes, which
/// is the inverted-obvious the `DoD` warns about.
///
/// **And it is token membership, not a substring.** The first version used
/// `ColumnTrait::contains`, an unanchored `LIKE '%eu%'`, so a row stored
/// `eur` or `aus,eu-central` matched a claim of `eu`. The pattern below is
/// separator-wrapped, so `,eu,` cannot be found inside `,eur,`.
#[test]
fn the_scope_predicate_is_token_membership_and_admits_the_empty_set() {
    let (sql, values) = scope_sql("eu");
    assert!(
        sql.contains(" OR "),
        "the predicate is a disjunction: {sql}"
    );
    assert!(sql.contains("= ''"), "the empty set is admitted: {sql}");
    // Four positional arms plus the unrestricted one: the whole value, the
    // first member, the last, and a middle one.
    assert!(
        values.iter().any(|v| v.contains("eu,%")),
        "the first-member arm is bound: {values:?}"
    );
    assert!(
        values.iter().any(|v| v.contains("%,eu,%")),
        "and the middle-member arm: {values:?}"
    );
    assert!(
        !values.iter().any(|v| v == "'%eu%'"),
        "no unanchored substring pattern survives: {values:?}"
    );
}

/// **A claim that is not a single token gets no containment arm at all.**
/// The substring form answered every restricted row for a claim of `%`;
/// here the predicate collapses to the unrestricted rows, which is the safe
/// direction and is asserted, not assumed.
#[test]
fn a_claim_that_is_not_a_token_is_refused_the_containment_arm() {
    for bad in ["%", "_", "eu,us", "", "a\\b"] {
        let (sql, values) = scope_sql(bad);
        assert!(
            !sql.contains("LIKE"),
            "claim {bad:?} must produce no pattern at all: {sql}"
        );
        assert_eq!(
            values,
            vec!["''".to_owned()],
            "only the unrestricted arm survives for {bad:?}"
        );
    }
}
