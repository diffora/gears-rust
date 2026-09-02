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
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, Condition, ConnectionTrait as _, EntityTrait as _};
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::{AccessScope, SecureUpdateExt as _};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    AssignmentWrite, AttributeCoordinate, CategoryWrite, DefinitionFlip, NewAttributeDefinition,
    NewCategory, attribute_definition_by_key, attribute_definitions, attribute_values_of,
    category_assignments, category_mutation_seq, category_parents, classify_assignment_write,
    definition_value_holders, delete_attribute_value, delete_metadata_key, delete_retired_category,
    flip_definition_state, insert_attribute_definition, insert_category, metadata_of,
    rename_category, replace_category_assignments, retire_category, retire_census,
    seed_well_known_definitions, upsert_attribute_value, upsert_metadata,
    write_category_display_value,
};
use crate::domain::taxonomy::{
    AssignmentRole, DefinitionState, REGISTRY_SEEDED_BY, WELL_KNOWN_SEEDS,
    definition_in_use_verdict, retire_verdict,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::product;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{
    NewEntityVersion, NewProduct, NewSku, VersionedEntityKind, discard_product_head,
    insert_entity_version, insert_product, insert_sku, publish_product_head,
};

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const BRAND: Uuid = Uuid::from_u128(0xb1_01);
const PRODUCT: Uuid = Uuid::from_u128(0xf0_01);
const CATEGORY_A: Uuid = Uuid::from_u128(0xca_01);
const CATEGORY_B: Uuid = Uuid::from_u128(0xca_02);
const DEFINITION: Uuid = Uuid::from_u128(0xde_01);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);
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

// -- The retire and delete guard (`inst-tx-retire-guard`) --

/// Walk one Product to `published` through the repository's own writers, so
/// the head-row guard judges the setup exactly as it judges a door's.
///
/// A hand-written `UPDATE` to `published` would have been shorter and wrong:
/// the guard admits the edge without a `published_version` bump, so the row
/// would sit in a state no door can produce -- `published` with
/// `published_version = 0` -- and the census would then be probed against a
/// head the gear cannot have.
async fn publish(conn: &impl toolkit_db::secure::DBRunner, scope: &AccessScope, product_id: Uuid) {
    insert_entity_version(
        conn,
        scope,
        NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: VersionedEntityKind::Product,
            entity_id: product_id,
            published_version: 1,
            content: r#"{"name":"Fibre 500"}"#.to_owned(),
            content_digest: (1..=32_u8).collect(),
            digest_version: 7,
            approval_ref: None,
            actor_ref: ACTOR,
            published_at: at(10),
        },
    )
    .await
    .expect("freeze version 1");
    publish_product_head(conn, scope, TENANT, product_id, 1, at(10))
        .await
        .expect("publish the head");
}

/// Move a published head on, one admitted edge at a time.
///
/// `published -> deprecated -> retired` are the only two edges to the states
/// the census must ignore, and each bumps `internal_revision` by exactly one
/// because the guard refuses anything else.
async fn move_head(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    product_id: Uuid,
    to: &str,
    next_revision: i64,
) {
    let moved = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(product::Column::LifecycleState, Expr::value(to))
        .col_expr(
            product::Column::InternalRevision,
            Expr::value(next_revision),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(at(11)))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(TENANT))
                .add(product::Column::ProductId.eq(product_id)),
        )
        .exec(conn)
        .await
        .expect("the guard admits this edge");
    assert_eq!(moved.rows_affected, 1, "this helper's own premise");
}

/// Seed a second Product, file it under `CATEGORY_A`, and answer its id.
async fn file_a_product_under_a(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    product_id: Uuid,
    name: &str,
) {
    let mut new = new_product(product_id, TENANT);
    new.name = name.to_owned();
    new.name_normalized = name.to_ascii_lowercase();
    insert_product(conn, scope, new)
        .await
        .expect("seed the holder");
    replace_category_assignments(
        conn,
        scope,
        TENANT,
        product_id,
        &[(CATEGORY_A, AssignmentRole::Primary)],
        at(10),
    )
    .await
    .expect("file it");
}

/// **`dod-retire-delete-guard`'s named `MUST`: a discarded draft holding a
/// link does not block the retire.**
///
/// And the assertion that makes it a probe rather than a coincidence: the
/// link row is read back and shown to still be there. Without that, a census
/// that returned nothing because the *assignment* had vanished would pass
/// this case while the rule it exists to hold went unmeasured.
#[tokio::test]
async fn a_discarded_draft_holding_a_link_does_not_block_the_retire() {
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
    .expect("file the draft under the category");

    // It blocks while it is a live draft -- the paired positive control,
    // without which the refusal below could be a census that reads nothing.
    let live = retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
        .await
        .expect("census");
    assert_eq!(live.referencing_products.len(), 1, "a draft blocks");

    // `expected_internal_revision` is 1: a freshly inserted draft is at
    // revision 1 and the discard is its first act.
    discard_product_head(&conn, &scope, TENANT, PRODUCT, 1, at(11))
        .await
        .expect("discard the draft");

    let after = retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
        .await
        .expect("census");
    assert!(
        after.referencing_products.is_empty(),
        "a discarded draft is terminal and must not block: {after:?}"
    );
    retire_verdict(&after).expect("the retire is admitted");

    assert_eq!(
        category_assignments(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read the links")
            .len(),
        1,
        "the link row is STILL THERE -- the guard read the Product's state, \
         not the row's presence, which is the DoD's own distinction"
    );
}

/// **Every non-terminal state blocks, and both terminal ones do not.**
///
/// One Product walked along its own admitted edges, the census re-read at
/// each. Asserting the roster this way rather than against
/// `TERMINAL_HEAD_STATES` is deliberate: the constant is what the statement
/// filters on, so comparing the census to it would be comparing the code to
/// itself.
#[tokio::test]
async fn the_census_counts_exactly_the_non_terminal_states() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    replace_category_assignments(
        &conn,
        &scope,
        TENANT,
        PRODUCT,
        &[(CATEGORY_A, AssignmentRole::Secondary)],
        at(10),
    )
    .await
    .expect("file it, in the SECONDARY role: the DoD says either role blocks");

    let blocked = |c: &crate::domain::taxonomy::RetireCensus| !c.referencing_products.is_empty();

    assert!(
        blocked(
            &retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
                .await
                .expect("census")
        ),
        "draft blocks"
    );

    publish(&conn, &scope, PRODUCT).await;
    assert!(
        blocked(
            &retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
                .await
                .expect("census")
        ),
        "published blocks"
    );

    move_head(&conn, &scope, PRODUCT, "deprecated", 3).await;
    assert!(
        blocked(
            &retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
                .await
                .expect("census")
        ),
        "deprecated blocks"
    );

    move_head(&conn, &scope, PRODUCT, "retired", 4).await;
    assert!(
        !blocked(
            &retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
                .await
                .expect("census")
        ),
        "retired is terminal and must not block"
    );
}

/// **An active child blocks; a retired one does not.**
///
/// `inst-ce-terminal` makes deletion a retired node's own exit, so a retired
/// child is on its way out rather than in use. A guard counting every child
/// would deadlock a depth-first retirement at its second step.
#[tokio::test]
async fn an_active_child_blocks_the_retire_and_a_retired_child_does_not() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let child = Uuid::from_u128(0xca_09);
    insert_category(
        &conn,
        &scope,
        NewCategory {
            tenant_id: TENANT,
            category_id: child,
            parent_id: Some(CATEGORY_A),
            name: "fibre",
            name_normalized: "fibre",
        },
        at(9),
    )
    .await
    .expect("insert the child")
    .expect("the name is free");

    let with_child = retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
        .await
        .expect("census");
    assert_eq!(with_child.active_children, vec!["fibre".to_owned()]);
    retire_verdict(&with_child).expect_err("an active child holds the parent");

    assert_eq!(
        retire_category(&conn, &scope, TENANT, child, at(10))
            .await
            .expect("retire the child"),
        CategoryWrite::Applied
    );

    let after = retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
        .await
        .expect("census");
    assert!(
        after.active_children.is_empty(),
        "a retired child does not block: {after:?}"
    );
    retire_verdict(&after).expect("the parent may now retire");
}

/// **The sample stops at `bound + 1`**, which is what lets the verdict say
/// *"at least N"* without a second counting statement.
#[tokio::test]
async fn the_holder_sample_reads_one_past_its_bound() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    for (n, id) in [0xf0_11, 0xf0_12, 0xf0_13, 0xf0_14].into_iter().enumerate() {
        file_a_product_under_a(&conn, &scope, Uuid::from_u128(id), &format!("Holder {n}")).await;
    }

    let census = retire_census(&conn, &scope, TENANT, CATEGORY_A, 2)
        .await
        .expect("census");
    assert_eq!(
        census.referencing_products.len(),
        3,
        "bound 2 reads 3 rows, so the caller can tell 'two' from 'more than two'"
    );
    assert_eq!(census.sample_bound, 2);
    let refusal = retire_verdict(&census).expect_err("held");
    assert!(refusal.detail.contains("at least 2"), "{refusal:?}");
}

/// **The retire is pinned at `active`**, so a peer that retired the node
/// between the caller's census and the write moves no second row, and the
/// caller answers a staleness refusal rather than reporting an act it did not
/// perform.
#[tokio::test]
async fn a_retire_is_pinned_at_active_and_the_second_one_matches_nothing() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    assert_eq!(
        retire_category(&conn, &scope, TENANT, CATEGORY_A, at(10))
            .await
            .expect("retire"),
        CategoryWrite::Applied
    );
    assert_eq!(
        retire_category(&conn, &scope, TENANT, CATEGORY_A, at(11))
            .await
            .expect("retire again"),
        CategoryWrite::Unmatched,
        "the pin is the state, so the second retire matches no row"
    );
}

/// **Deletion is the retired node's exit and nothing else's.**
///
/// `inst-ce-terminal` performs the single physical row removal this feature
/// owns, and only from `retired`. A delete filtered on the id alone would
/// take a live node with the same call.
#[tokio::test]
async fn only_a_retired_category_can_be_deleted() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    assert_eq!(
        delete_retired_category(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("attempt the delete"),
        CategoryWrite::Unmatched,
        "an active node is not deletable"
    );

    retire_category(&conn, &scope, TENANT, CATEGORY_A, at(10))
        .await
        .expect("retire");
    assert_eq!(
        delete_retired_category(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("delete"),
        CategoryWrite::Applied
    );
    assert!(
        category_parents(&conn, &scope, TENANT)
            .await
            .expect("read the tree")
            .iter()
            .all(|(id, _)| *id != CATEGORY_A),
        "the row is gone"
    );
}

/// **The census and the parent foreign key are not the same guard**, and the
/// module doc says so -- so it is measured rather than asserted.
///
/// A *retired* child does not appear in the census, so the verdict admits the
/// parent's retire and delete. The FK still refuses the delete, because the
/// child row points at the parent whatever state it is in. The two are
/// consistent: the census decides whether the act is admitted, the engine
/// decides whether it is possible, and the caller retires and deletes
/// depth-first.
#[tokio::test]
async fn a_retired_child_clears_the_census_and_the_foreign_key_still_refuses() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let conn = provider.conn().expect("scoped connection");

    let child = Uuid::from_u128(0xca_0a);
    insert_category(
        &conn,
        &scope,
        NewCategory {
            tenant_id: TENANT,
            category_id: child,
            parent_id: Some(CATEGORY_A),
            name: "fibre",
            name_normalized: "fibre",
        },
        at(9),
    )
    .await
    .expect("insert the child")
    .expect("the name is free");
    retire_category(&conn, &scope, TENANT, child, at(10))
        .await
        .expect("retire the child");

    let census = retire_census(&conn, &scope, TENANT, CATEGORY_A, 3)
        .await
        .expect("census");
    retire_verdict(&census).expect("a retired child does not hold the parent");

    retire_category(&conn, &scope, TENANT, CATEGORY_A, at(11))
        .await
        .expect("the parent retires");

    let refused = delete_retired_category(&conn, &scope, TENANT, CATEGORY_A).await;
    assert!(
        refused.is_err(),
        "fk_products_category_parent refuses while the child row exists: {refused:?}"
    );

    // Depth-first, and then the parent goes.
    delete_retired_category(&conn, &scope, TENANT, child)
        .await
        .expect("the child goes first");
    assert_eq!(
        delete_retired_category(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("and then the parent"),
        CategoryWrite::Applied
    );
}

// -- The definition removal operand (`dod-definition-lifecycle`) --

/// **The `DoD`'s both-ways probe, on its exact scenario.**
///
/// *"removal refused while a non-terminal head carries a value, and removal
/// **admitted** while only a frozen version carries one"*. So the Product is
/// walked all the way to `retired` through real edges -- published first, so a
/// frozen version genuinely exists and carries the value -- and the census is
/// read at each end.
///
/// The assertion that makes the second half a probe rather than a
/// coincidence: the `products_attribute_value` row is read back and shown to
/// still be there. A census that answered empty because the *value* had
/// vanished would pass while the rule went unmeasured, which is the same trap
/// the retire guard's own case carries.
#[tokio::test]
async fn a_terminal_heads_frozen_value_does_not_block_the_definitions_removal() {
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
    .expect("the head carries a value");

    let live = definition_value_holders(&conn, &scope, TENANT, DEFINITION, 3)
        .await
        .expect("census");
    assert_eq!(live.len(), 1, "a non-terminal head blocks the removal");
    definition_in_use_verdict(&live, 3).expect_err("refused while it is live");

    // published -> deprecated -> retired, so a frozen version exists and the
    // head is terminal.
    publish(&conn, &scope, PRODUCT).await;
    move_head(&conn, &scope, PRODUCT, "deprecated", 3).await;
    move_head(&conn, &scope, PRODUCT, "retired", 4).await;

    let after = definition_value_holders(&conn, &scope, TENANT, DEFINITION, 3)
        .await
        .expect("census");
    assert!(
        after.is_empty(),
        "only a frozen version carries it now, so the removal is admitted: {after:?}"
    );
    definition_in_use_verdict(&after, 3).expect("admitted");

    assert_eq!(
        attribute_values_of(&conn, &scope, TENANT, "product", PRODUCT)
            .await
            .expect("read the values")
            .len(),
        1,
        "the value row is STILL THERE -- the census read the head's state, not \
         the row's presence"
    );
}

/// **A SKU carries values too, and the census counts it.**
///
/// The key is polymorphic and the census reads three tables; a version that
/// read only `products_product` would answer *"unreferenced"* about every
/// definition the catalog uses on SKUs alone, which is most of them.
#[tokio::test]
async fn a_non_terminal_sku_carrying_a_value_blocks_the_removal() {
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

    let sku_id = Uuid::from_u128(0x5c_01);
    insert_sku(
        &conn,
        &scope,
        NewSku {
            sku_id,
            tenant_id: TENANT,
            product_id: PRODUCT,
            sku_code: "FIBRE-500-STD".to_owned(),
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: "principal:author-1".to_owned(),
            created_at: at(9),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("seed the sku");

    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        AttributeCoordinate {
            entity_kind: "sku",
            entity_id: sku_id,
            ..global(DEFINITION, sku_id)
        },
        "teal",
        at(10),
    )
    .await
    .expect("the sku carries a value");

    let census = definition_value_holders(&conn, &scope, TENANT, DEFINITION, 3)
        .await
        .expect("census");
    assert_eq!(census, vec!["FIBRE-500-STD".to_owned()]);
}

/// **An active category carrying a value blocks; a retired one does not.**
///
/// A category has no lifecycle state, so there is no terminal reading
/// available -- `design/02` §6 records that the removal guard *"counts an
/// active category as a value-carrying head"*, and this pins that behaviour so
/// a later reading of that row can see what it is changing.
#[tokio::test]
async fn an_active_category_carrying_a_value_blocks_the_removal() {
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

    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        AttributeCoordinate {
            entity_kind: "category",
            entity_id: CATEGORY_A,
            ..global(DEFINITION, CATEGORY_A)
        },
        "Connectivity",
        at(10),
    )
    .await
    .expect("the category carries a display value");

    assert_eq!(
        definition_value_holders(&conn, &scope, TENANT, DEFINITION, 3)
            .await
            .expect("census")
            .len(),
        1,
        "an active category counts"
    );

    retire_category(&conn, &scope, TENANT, CATEGORY_A, at(11))
        .await
        .expect("retire it");

    assert!(
        definition_value_holders(&conn, &scope, TENANT, DEFINITION, 3)
            .await
            .expect("census")
            .is_empty(),
        "a retired category does not"
    );
}

/// **Assignment rows roll back with the transaction they ride in.**
///
/// `dod-assignment-validators`: *"Assignment rows **MUST** land inside the
/// save door's transaction, and a rollback **MUST** leave neither the head
/// update nor the assignment rows."* This holds the half this strand owns --
/// that the write takes the caller's runner and has no transaction of its
/// own. The head-update half is the door's and is not measured here.
#[tokio::test]
async fn assignment_rows_roll_back_with_the_transaction_they_ride_in() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    seed_product_and_categories(&provider, &scope).await;
    let scope_for_mutation = scope.clone();

    let mutation = provider
        .transaction(move |tx| {
            Box::pin(async move {
                replace_category_assignments(
                    tx,
                    &scope_for_mutation,
                    TENANT,
                    PRODUCT,
                    &[(CATEGORY_A, AssignmentRole::Primary)],
                    at(10),
                )
                .await
                .map_err(|e| DbError::Other(anyhow::Error::msg(e.to_string())))?;

                Err::<(), DbError>(DbError::Other(anyhow::Error::msg(
                    "the save fails after its assignment write",
                )))
            })
        })
        .await;
    assert!(mutation.is_err(), "the save must roll back");

    let conn = provider.conn().expect("scoped connection");
    assert!(
        category_assignments(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("read the assignments")
            .is_empty(),
        "an assignment that survived its rolled-back save would file a Product \
         under a category the save never committed"
    );
}

// -- The category live-value door's token (`inst-av-category-branch`, P-D-50) --

/// **A held token writes the value and advances by exactly one.**
#[tokio::test]
async fn a_current_token_writes_the_display_value_and_advances_once() {
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

    let seq = category_mutation_seq(&conn, &scope, TENANT, CATEGORY_A)
        .await
        .expect("read the token")
        .expect("the row exists");
    assert_eq!(seq, 0, "a fresh category starts at zero");

    let next = write_category_display_value(
        &conn,
        &scope,
        TENANT,
        CATEGORY_A,
        seq,
        (DEFINITION, "Connectivity"),
        at(10),
    )
    .await
    .expect("no storage failure")
    .expect("the token was current");
    assert_eq!(next, 1);
    assert_eq!(
        category_mutation_seq(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("read")
            .expect("exists"),
        1
    );

    let values = attribute_values_of(&conn, &scope, TENANT, "category", CATEGORY_A)
        .await
        .expect("read the values");
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].value, "Connectivity");
}

/// **A stale token writes nothing -- and the value does not land.**
///
/// The negative half is the one that matters. A door that checked the token
/// and then wrote regardless, or wrote first and checked after, would pass an
/// assertion on the counter alone while the display value it was refusing had
/// already been committed.
#[tokio::test]
async fn a_stale_token_refuses_and_leaves_no_value_behind() {
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

    // A peer act moves the row: a rename is an act and advances the counter.
    rename_category(
        &conn,
        &scope,
        TENANT,
        CATEGORY_A,
        "Connectivity",
        "connectivity",
        at(10),
    )
    .await
    .expect("no storage failure")
    .expect("the name is free");

    // Our door still holds the token it read before the rename.
    let refusal = write_category_display_value(
        &conn,
        &scope,
        TENANT,
        CATEGORY_A,
        0,
        (DEFINITION, "Connectivity"),
        at(11),
    )
    .await
    .expect("no storage failure")
    .expect_err("the token is stale");
    assert_eq!((refusal.expected, refusal.found), (0, 1));

    assert!(
        attribute_values_of(&conn, &scope, TENANT, "category", CATEGORY_A)
            .await
            .expect("read the values")
            .is_empty(),
        "a refused write must leave no value"
    );
    assert_eq!(
        category_mutation_seq(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("read")
            .expect("exists"),
        1,
        "and must not advance the counter either"
    );
}

/// **The counter counts acts, not row writes** -- P-D-50's own words.
///
/// Writing a category's value through the plain store path leaves the counter
/// alone; only the door's act advances it. Without this, a counter bumped by
/// any write of any row would change under an approval subject built from an
/// act identity, and the approved retry would render a different subject --
/// which is the exact failure P-D-50 names.
#[tokio::test]
async fn a_non_door_row_write_does_not_advance_the_act_counter() {
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

    upsert_attribute_value(
        &conn,
        &scope,
        TENANT,
        AttributeCoordinate {
            entity_kind: "category",
            entity_id: CATEGORY_A,
            ..global(DEFINITION, CATEGORY_A)
        },
        "written around the door",
        at(10),
    )
    .await
    .expect("write the value directly");

    assert_eq!(
        category_mutation_seq(&conn, &scope, TENANT, CATEGORY_A)
            .await
            .expect("read")
            .expect("exists"),
        0,
        "the value moved and the act counter did not"
    );
}

/// **A vanished row answers the same refusal**, with the sentinel counter so
/// the door can tell it apart and prefer a 404 after re-reading.
#[tokio::test]
async fn a_missing_category_answers_the_token_refusal_with_the_sentinel() {
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

    let refusal = write_category_display_value(
        &conn,
        &scope,
        TENANT,
        Uuid::from_u128(0xca_ff),
        0,
        (DEFINITION, "Nothing"),
        at(10),
    )
    .await
    .expect("no storage failure")
    .expect_err("there is no such category");
    assert_eq!(refusal.found, -1, "the sentinel says the row is not there");
}

// -- The well-known seeds (**P-D-100**, as amended by **P-D-104**) --

/// **An empty roster is seeded, marked `registry`, with `imageUri`
/// non-localized.**
///
/// The rows are asserted against `WELL_KNOWN_SEEDS` itself rather than
/// against a second list written here: a literal roster in the test would be
/// exactly the disagreement the single definition site exists to prevent.
///
/// **Seeding is a write and the roster read is a read.** P-D-104 separated the
/// two -- a lazy read-through made a `GET` mutate -- so this calls the writer
/// and reads back, which is what the door does.
#[tokio::test]
async fn an_empty_roster_is_seeded_with_the_well_known_five() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    assert!(
        attribute_definitions(&conn, &scope, TENANT)
            .await
            .expect("read")
            .is_empty(),
        "this case's own premise: nothing is seeded yet"
    );

    seed_well_known_definitions(&conn, &scope, TENANT, at(9))
        .await
        .expect("seed");
    let seeded = attribute_definitions(&conn, &scope, TENANT)
        .await
        .expect("read back");

    assert_eq!(
        seeded.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(),
        {
            let mut keys: Vec<&str> = WELL_KNOWN_SEEDS.iter().map(|s| s.key).collect();
            keys.sort_unstable();
            keys
        },
        "the roster is WELL_KNOWN_SEEDS', ordered by key"
    );
    for definition in &seeded {
        let seed = WELL_KNOWN_SEEDS
            .iter()
            .find(|s| s.key == definition.key)
            .expect("every row came from the roster");
        assert_eq!(definition.seeded_by.as_deref(), Some(REGISTRY_SEEDED_BY));
        assert_eq!(definition.state, DefinitionState::Active);
        assert_eq!(definition.localized, seed.localized, "{}", seed.key);
        assert_eq!(definition.value_type, seed.value_type, "{}", seed.key);
    }
}

/// **A read does not seed.** P-D-104's whole point: a `GET` of the roster must
/// not write, or a read-only replica breaks and the first reader of a tenant
/// pays for a write it did not ask for.
#[tokio::test]
async fn reading_the_roster_writes_nothing() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    for _ in 0..3 {
        assert!(
            attribute_definitions(&conn, &scope, TENANT)
                .await
                .expect("read")
                .is_empty(),
            "the read is pure; three of them leave the roster empty"
        );
    }
}

/// **Seeding twice adds nothing, and never undoes a deprecation.**
///
/// The door calls the writer on every content save that names an attribute,
/// so it must be idempotent -- the key index is what makes it so, and the
/// conflict is swallowed. The second half is the sharper one: a tenant that
/// has **deprecated** one of the five does not get it back. Re-materialising a
/// definition an operator deliberately moved out of the way would undo their
/// act, and the state flip is the only removal there is, so they would have no
/// way left to say no.
#[tokio::test]
async fn seeding_twice_adds_nothing_and_never_undoes_a_deprecation() {
    let provider = harness().await;
    let scope = AccessScope::for_tenant(TENANT);
    let conn = provider.conn().expect("scoped connection");

    seed_well_known_definitions(&conn, &scope, TENANT, at(9))
        .await
        .expect("seed");
    let first = attribute_definitions(&conn, &scope, TENANT)
        .await
        .expect("read");
    assert_eq!(first.len(), WELL_KNOWN_SEEDS.len());

    let display = first
        .iter()
        .find(|d| d.key == "displayName")
        .expect("the roster carries it");
    flip_definition_state(
        &conn,
        &scope,
        TENANT,
        display.definition_id,
        DefinitionFlip {
            expected: DefinitionState::Active,
            to: DefinitionState::Deprecated,
        },
        at(10),
    )
    .await
    .expect("the operator deprecates one");

    seed_well_known_definitions(&conn, &scope, TENANT, at(11))
        .await
        .expect("the door calls this again on the next such write");
    let second = attribute_definitions(&conn, &scope, TENANT)
        .await
        .expect("read again");
    assert_eq!(
        second.len(),
        WELL_KNOWN_SEEDS.len(),
        "no sixth row: the read-through did not fire on a non-empty roster"
    );
    assert_eq!(
        second
            .iter()
            .find(|d| d.key == "displayName")
            .expect("still there")
            .state,
        DefinitionState::Deprecated,
        "the operator's act stands"
    );
}

/// **Each tenant is seeded for itself**, which is the whole reason a migration
/// alone cannot do this: the rows are five *per tenant*, not five in the
/// database.
#[tokio::test]
async fn the_seeds_are_per_tenant() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");

    seed_well_known_definitions(&conn, &AccessScope::for_tenant(TENANT), TENANT, at(9))
        .await
        .expect("seed the first tenant");
    assert!(
        attribute_definitions(&conn, &AccessScope::for_tenant(OTHER_TENANT), OTHER_TENANT)
            .await
            .expect("read")
            .is_empty(),
        "seeding one tenant seeds no other"
    );

    seed_well_known_definitions(
        &conn,
        &AccessScope::for_tenant(OTHER_TENANT),
        OTHER_TENANT,
        at(9),
    )
    .await
    .expect("seed the second tenant");
    let other = attribute_definitions(&conn, &AccessScope::for_tenant(OTHER_TENANT), OTHER_TENANT)
        .await
        .expect("read it back");
    assert_eq!(other.len(), WELL_KNOWN_SEEDS.len());
}
