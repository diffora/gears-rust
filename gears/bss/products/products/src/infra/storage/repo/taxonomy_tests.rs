//! `products_product_category` / `products_attribute_definition` /
//! `products_attribute_value` / `products_metadata` repository tests, against
//! the executed `SQLite` mirror.
//!
//! # Why these live here and not in `repo_tests.rs`
//!
//! The crate's convention is that a module declares its test module at its
//! own bottom — `domain/taxonomy.rs`, `domain/live_op.rs` and
//! `infra/increment.rs` all do. Every repository test so far has instead
//! landed in one 4 336-line `repo_tests.rs` shared by every aggregate. This
//! file introduces the per-module convention into `repo/` for the first time,
//! which keeps three strands' repository tests off one another's lines. That
//! is a small structural decision and it is registered for the lead to
//! ratify rather than taken silently.
//!
//! # Only the `SQLite` mirror is executed
//!
//! As in `repo_tests.rs`: no case here runs a Postgres statement, so both
//! partial unique indexes and the definition table's `BEFORE DELETE` trigger
//! are measured on the `SQLite` half, resting on `migrations_tests.rs`
//! asserting the two halves clause for clause. The Postgres tier is a
//! separate gate.
//!
//! # What is asserted and what is only seeded
//!
//! Every fixture below is read back through the surface that wrote it. A
//! seeded row nothing reads proves only that an `INSERT` parsed.
#![allow(clippy::expect_used)]

use chrono::{TimeZone, Utc};
use sea_orm::ConnectionTrait as _;
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    AssignmentWrite, AttributeCoordinate, DefinitionFlip, NewAttributeDefinition, NewCategory,
    attribute_definition_by_key, attribute_definitions, attribute_values_of, category_assignments,
    classify_assignment_write, delete_attribute_value, delete_metadata_key, flip_definition_state,
    insert_attribute_definition, insert_category, metadata_of, replace_category_assignments,
    upsert_attribute_value, upsert_metadata,
};
use crate::domain::taxonomy::{AssignmentRole, DefinitionState};
use crate::infra::storage::RepoError;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{NewProduct, insert_product};

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const BRAND: Uuid = Uuid::from_u128(0xb1_01);
const PRODUCT: Uuid = Uuid::from_u128(0xf0_01);
const CATEGORY_A: Uuid = Uuid::from_u128(0xca_01);
const CATEGORY_B: Uuid = Uuid::from_u128(0xca_02);
const DEFINITION: Uuid = Uuid::from_u128(0xde_01);
const OTHER_DEFINITION: Uuid = Uuid::from_u128(0xde_02);

/// A pinned one-connection in-memory `SQLite` pool, for the reason
/// `repo_tests::harness` gives: a larger pool hands each checkout its own
/// empty database and the migrated tables appear to vanish.
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
    Utc.with_ymd_and_hms(2026, 9, 2, hour, 0, 0).unwrap()
}

fn new_product(product_id: Uuid, tenant_id: Uuid) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id,
        brand_id: BRAND,
        name: "Fibre 500".to_owned(),
        name_normalized: "fibre 500".to_owned(),
        product_code: None,
        region_scope: String::new(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        cloned_from: None,
        cloned_from_version: None,
    }
}

fn new_category(category_id: Uuid, tenant_id: Uuid, name: &str) -> NewCategory<'_> {
    NewCategory {
        tenant_id,
        category_id,
        parent_id: None,
        name,
        name_normalized: name,
    }
}

/// An unrestricted, localized definition — the shape most cases need.
///
/// Both scope columns are `""`, which under P-D-39 is **unrestricted**, not
/// empty. A case that wants a restricted definition builds its own.
fn definition(definition_id: Uuid, tenant_id: Uuid, key: &str) -> NewAttributeDefinition<'_> {
    NewAttributeDefinition {
        tenant_id,
        definition_id,
        key,
        value_type: "localized_string",
        localized: true,
        region_scope: "",
        brand_scope: "",
        seeded_by: None,
    }
}

fn global(definition_id: Uuid, entity_id: Uuid) -> AttributeCoordinate<'static> {
    AttributeCoordinate {
        entity_kind: "product",
        entity_id,
        definition_id,
        locale: "",
        region: "",
        brand: "",
    }
}

/// Seed a Product and two root categories the assignment cases need — both
/// FKs on `products_product_category` are real.
async fn seed_product_and_categories(provider: &DBProvider<DbError>, scope: &AccessScope) {
    let conn = provider.conn().expect("scoped connection");
    insert_product(&conn, scope, new_product(PRODUCT, TENANT))
        .await
        .expect("seed the product the assignments hang off");
    for (id, name) in [(CATEGORY_A, "connectivity"), (CATEGORY_B, "hardware")] {
        insert_category(&conn, scope, new_category(id, TENANT, name), at(9))
            .await
            .expect("insert category")
            .expect("the name is free");
    }
}

/// **`dod-category-assignment-table`'s last MUST, which nothing else arms.**
///
/// *"The Foundation's entity tables **MUST NOT** gain inline category
/// columns."* That clause is true at `HEAD` and is **asserted nowhere**: it is
/// stated in two module docs — this migration's and
/// `entity/product_category.rs`'s — and a doc comment refuses nothing. The
/// day someone adds `products_product.primary_category_id` as a denormalized
/// read convenience, the assignment table stops being the single source of
/// truth and every test in this file still passes.
///
/// So the probe reads the **engine's own** column list rather than the entity
/// or the migration source: an entity iteration would miss a column added to
/// the DDL alone, and a text scan of the migration would redden on a prose
/// mention of the word. `migrations_tests.rs` reads these two tables but
/// asserts only two named columns' presence, never a whole roster — measured,
/// not assumed — so nothing there covers this either.
#[tokio::test]
async fn the_foundations_head_tables_carry_no_inline_category_column() {
    let mut opts = sea_orm::ConnectOptions::new("sqlite::memory:");
    opts.max_connections(1).min_connections(1);
    let db = sea_orm::Database::connect(opts)
        .await
        .expect("connect in-memory sqlite");
    Migrator::up(&db, None).await.expect("boot the chain");

    for table in ["products_product", "products_sku"] {
        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                format!("SELECT name FROM pragma_table_info('{table}') ORDER BY name"),
            ))
            .await
            .expect("the engine reports its own columns");
        let columns: Vec<String> = rows
            .iter()
            .map(|row| {
                row.try_get::<String>("", "name")
                    .expect("pragma_table_info carries a name column")
            })
            .collect();
        // The positive control. `pragma_table_info` on a table that does not
        // exist answers an empty set rather than failing, so without this the
        // probe below would pass by reading nothing at all -- a typo in the
        // table name would make it permanently green.
        assert!(
            columns.contains(&"tenant_id".to_owned())
                && columns.contains(&"lifecycle_state".to_owned()),
            "the pragma reached {table} and read its real roster: {columns:?}"
        );
        assert!(
            !columns.iter().any(|c| c.contains("categor")),
            "{table} gained an inline category column ({columns:?}): \
             products_product_category is the single source of truth"
        );
    }
}

// -- `products_product_category` --

/// **The assignment set round-trips with both roles and its order.**
///
/// If the write mapped `role` wrong, dropped `assigned_at`, or the read
/// parsed the role from the wrong column, this is the only case that would
/// notice: `replace_category_assignments` returns no row it did not write.
#[tokio::test]
async fn an_assignment_set_reads_back_with_both_roles_intact() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let written = replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[
            (CATEGORY_A, AssignmentRole::Primary),
            (CATEGORY_B, AssignmentRole::Secondary),
        ],
        at(10),
    )
    .await
    .expect("write the assignment set");
    assert_eq!(written, AssignmentWrite::Applied);

    let read = category_assignments(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read the assignment set");
    assert_eq!(read.len(), 2, "both rows landed");
    let primary = read
        .iter()
        .find(|a| a.role == AssignmentRole::Primary)
        .expect("the primary is stored as primary");
    assert_eq!(primary.category_id, CATEGORY_A);
    assert_eq!(primary.assigned_at, at(10));
    assert_eq!(
        read.iter()
            .filter(|a| a.role == AssignmentRole::Secondary)
            .map(|a| a.category_id)
            .collect::<Vec<_>>(),
        vec![CATEGORY_B]
    );
}

/// **At-most-one-primary is the index refusing, not this code checking.**
///
/// The `DoD` requires it be *"an index rather than an application convention"*,
/// so the payload names two primaries and nothing reads before writing.
#[tokio::test]
async fn a_second_primary_is_refused_by_the_partial_index() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let written = replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[
            (CATEGORY_A, AssignmentRole::Primary),
            (CATEGORY_B, AssignmentRole::Primary),
        ],
        at(10),
    )
    .await
    .expect("the refusal is an outcome, not a storage error");
    assert_eq!(written, AssignmentWrite::PrimaryConflict);
}

/// **The primary conflict is not read as a duplicate category.**
///
/// `uq_products_product_category_primary` contains
/// `uq_products_product_category` as a prefix, so a classifier that tested
/// the shorter name first would report every second primary as a duplicate.
/// The two cases are asserted against each other rather than each alone.
#[tokio::test]
async fn the_primary_conflict_is_not_read_as_a_duplicate() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let two_primaries = replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[
            (CATEGORY_A, AssignmentRole::Primary),
            (CATEGORY_B, AssignmentRole::Primary),
        ],
        at(10),
    )
    .await
    .expect("classified");

    let one_category_twice = replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[
            (CATEGORY_A, AssignmentRole::Primary),
            (CATEGORY_A, AssignmentRole::Secondary),
        ],
        at(10),
    )
    .await
    .expect("classified");

    assert_eq!(two_primaries, AssignmentWrite::PrimaryConflict);
    assert_eq!(one_category_twice, AssignmentWrite::DuplicateCategory);
    assert_ne!(
        two_primaries, one_category_twice,
        "the two indexes must not collapse into one outcome"
    );
}

/// **Replace removes what the payload dropped.** A merge would leave a
/// category the operator deleted still filed, and the door has no second
/// call with which to notice.
#[tokio::test]
async fn replacing_a_set_drops_the_assignments_the_payload_omits() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[
            (CATEGORY_A, AssignmentRole::Primary),
            (CATEGORY_B, AssignmentRole::Secondary),
        ],
        at(10),
    )
    .await
    .expect("the first set");

    replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[(CATEGORY_B, AssignmentRole::Primary)],
        at(11),
    )
    .await
    .expect("the second set");

    let read = category_assignments(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read back");
    assert_eq!(read.len(), 1, "the dropped assignment is gone");
    assert_eq!(read[0].category_id, CATEGORY_B);
    assert_eq!(
        read[0].role,
        AssignmentRole::Primary,
        "a role change survives the replace: the old primary was cleared first"
    );
}

/// **An empty payload clears the set** — the case a merge-shaped write can
/// never express, and the one an operator uses to unfile a Product.
#[tokio::test]
async fn an_empty_payload_clears_the_whole_set() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[(CATEGORY_A, AssignmentRole::Primary)],
        at(10),
    )
    .await
    .expect("the first set");

    let cleared = replace_category_assignments(&conn, &scope, TENANT, PRODUCT, &[], at(11))
        .await
        .expect("clear");
    assert_eq!(cleared, AssignmentWrite::Applied);
    assert!(
        category_assignments(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read back")
            .is_empty()
    );
}

/// **The Postgres arm of the classifier, which no statement here executes.**
///
/// This suite runs on the `SQLite` mirror, so the only message shape the
/// cases above exercise is the column-list one. The name-based arm is what
/// runs in production on Postgres, and leaving it unmeasured would ship the
/// branch whose ordering trap the classifier's own doc warns about with no
/// test against it at all.
///
/// So the classifier is called directly on Postgres's own wording. This is a
/// test of the **classifier**, not of Postgres: it proves the branch reads the
/// two constraint names apart, and it would still pass if the server never
/// produced these strings. Executing them belongs in
/// `tests/postgres_taxonomy_schema.rs`, a file outside this strand's granted
/// set, so that probe is handed to the lead rather than written here.
#[test]
fn the_classifier_reads_postgres_constraint_names_apart() {
    let pg = |constraint: &str| {
        RepoError::Db(format!(
            "duplicate key value violates unique constraint \"{constraint}\""
        ))
    };
    assert_eq!(
        classify_assignment_write(&pg("uq_products_product_category_primary")),
        Some(AssignmentWrite::PrimaryConflict),
        "the partial index, whose name CONTAINS the table-level one"
    );
    assert_eq!(
        classify_assignment_write(&pg("uq_products_product_category")),
        Some(AssignmentWrite::DuplicateCategory)
    );
    assert_eq!(
        classify_assignment_write(&pg("products_product_category_pkey")),
        Some(AssignmentWrite::DuplicateCategory),
        "the same category in the same role twice is a duplicate, not a \
         storage failure the door has to render as a 500"
    );
}

/// **A failure that is not a uniqueness one is not classified at all.**
///
/// Without this, a classifier gated only on the table name would widen every
/// foreign-key and CHECK refusal into a duplicate-assignment outcome, and a
/// door told "duplicate" for a missing category would name the wrong thing to
/// the operator.
#[test]
fn a_non_uniqueness_failure_is_left_as_a_storage_error() {
    for message in [
        "foreign key constraint failed",
        "CHECK constraint failed: chk_products_product_category_role",
        "no such table: products_product_category",
    ] {
        assert_eq!(
            classify_assignment_write(&RepoError::Db(message.to_owned())),
            None,
            "`{message}` is not a uniqueness refusal"
        );
    }
}

// -- `products_attribute_definition` --

/// **A definition round-trips with every column intact**, including the two
/// scope columns whose empty string means *unrestricted* rather than empty
/// (P-D-39) and the `seeded_by` marker.
#[tokio::test]
async fn a_definition_reads_back_with_every_column_intact() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    let created = insert_attribute_definition(
        &conn,
        &scope,
        NewAttributeDefinition {
            tenant_id: TENANT,
            definition_id: DEFINITION,
            key: "displayName",
            value_type: "localized_string",
            localized: true,
            region_scope: "eu,apac",
            brand_scope: "",
            seeded_by: Some("registry"),
        },
        at(9),
    )
    .await
    .expect("define");

    let found = attribute_definition_by_key(&conn, &scope, TENANT, "displayName")
        .await
        .expect("read")
        .expect("the row exists");

    assert_eq!(found, created);
    assert_eq!(found.definition_id, DEFINITION);
    assert_eq!(found.value_type, "localized_string");
    assert!(found.localized);
    assert_eq!(found.region_scope, "eu,apac");
    assert_eq!(
        found.brand_scope, "",
        "an empty scope is UNRESTRICTED, and must survive the round trip as \
         the empty string rather than becoming a null or a literal"
    );
    assert_eq!(
        found.state,
        DefinitionState::Active,
        "a definition is born active"
    );
    assert_eq!(found.seeded_by.as_deref(), Some("registry"));
}

/// **A key is unique per tenant and the index decides it**, and the same key
/// in another tenant is a different definition.
#[tokio::test]
async fn a_key_is_unique_per_tenant_and_free_in_another() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("the first definition");

    let clash = insert_attribute_definition(
        &conn,
        &scope,
        definition(OTHER_DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await;
    assert!(clash.is_err(), "the tenant-unique key index refuses it");

    let other_scope = AccessScope::for_tenant(OTHER_TENANT);
    insert_attribute_definition(
        &conn,
        &other_scope,
        definition(OTHER_DEFINITION, OTHER_TENANT, "colour"),
        at(9),
    )
    .await
    .expect("the same key in another tenant is a different definition");
}

/// **`removed` is reached by a flip and by nothing else**, which is the
/// `DoD`'s own clause. The store offers no delete for this table, and the
/// tombstone is still readable afterwards — a value on a terminal head keeps
/// resolving past its definition's removal.
#[tokio::test]
async fn removed_is_reached_by_a_flip_and_the_tombstone_still_reads() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    for (expected, to) in [
        (DefinitionState::Active, DefinitionState::Deprecated),
        (DefinitionState::Deprecated, DefinitionState::Removed),
    ] {
        assert!(
            flip_definition_state(
                &conn,
                &scope,
                TENANT,
                DEFINITION,
                DefinitionFlip { expected, to },
                at(10),
            )
            .await
            .expect("flip"),
            "{expected:?} -> {to:?} moved one row"
        );
    }

    let tombstone = attribute_definition_by_key(&conn, &scope, TENANT, "colour")
        .await
        .expect("read")
        .expect("the row survives its removal");
    assert_eq!(tombstone.state, DefinitionState::Removed);
}

/// **A stale pin moves no row.** The flip carries the state the caller's
/// `GovernedLiveOp` read, so a peer's flip in between leaves
/// `rows_affected = 0` and the door answers `STALE_LIVE_OP` rather than
/// absorbing the race.
///
/// The paired positive control is above: without it, a `flip` that always
/// answered `false` would pass this case alone.
#[tokio::test]
async fn a_flip_pinned_at_a_state_the_row_has_left_moves_nothing() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    // A peer deprecates it.
    flip_definition_state(
        &conn,
        &scope,
        TENANT,
        DEFINITION,
        DefinitionFlip {
            expected: DefinitionState::Active,
            to: DefinitionState::Deprecated,
        },
        at(10),
    )
    .await
    .expect("the peer's flip");

    // Our own op still believes it is active.
    assert!(
        !flip_definition_state(
            &conn,
            &scope,
            TENANT,
            DEFINITION,
            DefinitionFlip {
                expected: DefinitionState::Active,
                to: DefinitionState::Removed,
            },
            at(11),
        )
        .await
        .expect("no storage failure"),
        "a stale pin must move nothing"
    );

    assert_eq!(
        attribute_definition_by_key(&conn, &scope, TENANT, "colour")
            .await
            .expect("read")
            .expect("exists")
            .state,
        DefinitionState::Deprecated,
        "the peer's state stands"
    );
}

/// **The roster read returns every state, tombstones included**, ordered by
/// key. Filtering `removed` here would make a terminal head's value
/// unresolvable, which is the property `inst-de-edge-remove` exists to keep.
#[tokio::test]
async fn the_roster_carries_tombstones_and_is_ordered_by_key() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    for (id, key) in [(DEFINITION, "zeta"), (OTHER_DEFINITION, "alpha")] {
        insert_attribute_definition(&conn, &scope, definition(id, TENANT, key), at(9))
            .await
            .expect("define");
    }
    for (expected, to) in [
        (DefinitionState::Active, DefinitionState::Deprecated),
        (DefinitionState::Deprecated, DefinitionState::Removed),
    ] {
        flip_definition_state(
            &conn,
            &scope,
            TENANT,
            DEFINITION,
            DefinitionFlip { expected, to },
            at(10),
        )
        .await
        .expect("flip zeta to a tombstone");
    }

    let roster = attribute_definitions(&conn, &scope, TENANT)
        .await
        .expect("read the roster");
    assert_eq!(
        roster.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "zeta"],
        "ordered by key, and the tombstone is present"
    );
    assert_eq!(roster[1].state, DefinitionState::Removed);
}

// -- `products_attribute_value` --

/// **A value round-trips at the global coordinate**, which P-D-88 arm 2
/// spells `("", "", "")` rather than three nulls — so the key is total and
/// the `UNIQUE` actually constrains the one row `dod-default-locale` makes
/// mandatory.
#[tokio::test]
async fn a_value_round_trips_at_the_global_coordinate() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");
    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        global(DEFINITION, PRODUCT),
        "teal",
        at(10),
    )
    .await
    .expect("write the global value");

    let values = attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
        .await
        .expect("read back");
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].value, "teal");
    assert_eq!(
        (
            values[0].locale.as_str(),
            values[0].region.as_str(),
            values[0].brand.as_str()
        ),
        ("", "", ""),
        "the global coordinate is three empty strings, not three nulls"
    );
    assert_eq!(values[0].updated_at, at(10));
}

/// **The same coordinate twice is one row overwritten, not two rows.** Both
/// engines treat NULLs as distinct, which is why P-D-88 arm 2 made these
/// columns `NOT NULL` — a nullable tuple would have left exactly this write
/// unconstrained by the very key declared to constrain it.
#[tokio::test]
async fn the_same_coordinate_written_twice_is_one_row() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");
    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        global(DEFINITION, PRODUCT),
        "teal",
        at(10),
    )
    .await
    .expect("first write");
    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        global(DEFINITION, PRODUCT),
        "amber",
        at(11),
    )
    .await
    .expect("second write");

    let values = attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
        .await
        .expect("read back");
    assert_eq!(values.len(), 1, "one coordinate, one row");
    assert_eq!(values[0].value, "amber", "the later write stands");
    assert_eq!(values[0].updated_at, at(11));
}

/// **A locale coordinate is a different row from the global one**, and the
/// read's order is total over the four coordinate columns.
#[tokio::test]
async fn the_locale_coordinates_are_distinct_rows_in_a_total_order() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");
    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    for (locale, region, brand, value) in [
        ("", "", "", "global"),
        ("fr-FR", "", "", "locale"),
        ("fr-FR", "eu", "", "locale+region"),
        ("fr-FR", "eu", "acme", "locale+region+brand"),
    ] {
        upsert_attribute_value(
            &conn,
            &scope,
            TENANT,
            AttributeCoordinate {
                entity_kind: "product",
                entity_id: PRODUCT,
                definition_id: DEFINITION,
                locale,
                region,
                brand,
            },
            value,
            at(10),
        )
        .await
        .expect("write one coordinate");
    }

    let values = attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
        .await
        .expect("read back");
    assert_eq!(values.len(), 4, "four coordinates, four rows");
    assert_eq!(
        values.iter().map(|v| v.value.as_str()).collect::<Vec<_>>(),
        vec!["global", "locale", "locale+region", "locale+region+brand"],
        "ordered by definition, then locale, region and brand -- the empty \
         string sorting first is what puts the global row at the head"
    );
}

/// **A `category` row is a value like any other**, which is H2's fix: for
/// category rows this table *is* the live state, with no freeze-copy. The
/// entity-kind column admits it because §7 row 20 is live and the migration
/// pinned the column to non-emptiness rather than to a roster.
#[tokio::test]
async fn a_category_carries_its_own_values_beside_a_products() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");
    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "displayName"),
        at(9),
    )
    .await
    .expect("define");

    for (kind, id, value) in [
        ("product", PRODUCT, "Fibre 500"),
        ("category", CATEGORY_A, "Connectivity"),
    ] {
        upsert_attribute_value(
            &conn,
            &scope,
            TENANT,
            AttributeCoordinate {
                entity_kind: kind,
                entity_id: id,
                definition_id: DEFINITION,
                locale: "",
                region: "",
                brand: "",
            },
            value,
            at(10),
        )
        .await
        .expect("write");
    }

    let category_values = attribute_values_of(&conn, &scope, TENANT, "category", CATEGORY_A)
        .await
        .expect("read the category's values");
    assert_eq!(category_values.len(), 1);
    assert_eq!(category_values[0].value, "Connectivity");
    assert_eq!(
        attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
            .await
            .expect("read the product's values")
            .len(),
        1,
        "the two kinds do not read each other's rows"
    );
}

/// **A value against a definition the tenant never declared is refused** by
/// the definition FK, so an unknown definition cannot be smuggled in as a
/// value coordinate.
#[tokio::test]
async fn a_value_against_an_undeclared_definition_is_refused() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let refused = upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        global(DEFINITION, PRODUCT),
        "teal",
        at(10),
    )
    .await;
    assert!(
        refused.is_err(),
        "fk_products_attribute_value_definition refuses it"
    );
}

/// **Removing a value removes exactly its coordinate**, and answers `false`
/// where none stood there — the operand a door needs to tell "cleared" from
/// "there was nothing to clear".
#[tokio::test]
async fn removing_a_value_takes_its_coordinate_and_no_neighbour() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");
    insert_attribute_definition(
        &conn,
        &scope,
        definition(DEFINITION, TENANT, "colour"),
        at(9),
    )
    .await
    .expect("define");

    let fr = AttributeCoordinate {
        locale: "fr-FR",
        ..global(DEFINITION, PRODUCT)
    };
    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        global(DEFINITION, PRODUCT),
        "teal",
        at(10),
    )
    .await
    .expect("the global value");
    upsert_attribute_value(&conn, &scope, TENANT, fr, "sarcelle", at(10))
        .await
        .expect("the French value");

    assert!(
        delete_attribute_value(&conn, &scope, TENANT, fr)
            .await
            .expect("remove"),
        "the French row was there"
    );
    assert!(
        !delete_attribute_value(&conn, &scope, TENANT, fr)
            .await
            .expect("remove again"),
        "a second removal answers false rather than pretending"
    );

    let left = attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
        .await
        .expect("read back");
    assert_eq!(left.len(), 1, "the global row is untouched");
    assert_eq!(left[0].value, "teal");
}

// -- `products_metadata` --

/// **A metadata key round-trips and an overwrite keeps `created_at`.**
///
/// The column means *when this key first appeared*; an upsert that rewrote it
/// would silently make it a second copy of `updated_at`, and nothing else in
/// the gear would notice.
#[tokio::test]
async fn a_metadata_key_round_trips_and_an_overwrite_keeps_created_at() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    upsert_metadata(
        &conn,
        &scope,
        TENANT,
        "product",
        PRODUCT,
        ("owner", "team-a"),
        at(10),
    )
    .await
    .expect("first write");
    upsert_metadata(
        &conn,
        &scope,
        TENANT,
        "product",
        PRODUCT,
        ("owner", "team-b"),
        at(11),
    )
    .await
    .expect("overwrite");

    let map = metadata_of(&conn, &scope, TENANT, "product", PRODUCT)
        .await
        .expect("read back");
    assert_eq!(map.len(), 1, "one key, one row");
    assert_eq!(map[0].value, "team-b");
    assert_eq!(map[0].created_at, at(10), "created_at is the first write's");
    assert_eq!(map[0].updated_at, at(11));
}

/// **The map is per key and ordered**, and a removal answers whether a row
/// stood there.
#[tokio::test]
async fn the_metadata_map_is_keyed_ordered_and_removable() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    for (key, value) in [("zone", "eu"), ("owner", "team-a")] {
        upsert_metadata(
            &conn,
            &scope,
            TENANT,
            "product",
            PRODUCT,
            (key, value),
            at(10),
        )
        .await
        .expect("write");
    }

    assert_eq!(
        metadata_of(&conn, &scope, TENANT, "product", PRODUCT)
            .await
            .expect("read")
            .iter()
            .map(|e| e.key.as_str())
            .collect::<Vec<_>>(),
        vec!["owner", "zone"],
        "ordered by key"
    );

    assert!(
        delete_metadata_key(&conn, &scope, TENANT, "product", PRODUCT, "zone")
            .await
            .expect("remove")
    );
    assert!(
        !delete_metadata_key(&conn, &scope, TENANT, "product", PRODUCT, "zone")
            .await
            .expect("remove again"),
        "a second removal answers false"
    );
    assert_eq!(
        metadata_of(&conn, &scope, TENANT, "product", PRODUCT)
            .await
            .expect("read")
            .len(),
        1
    );
}
