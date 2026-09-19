//! The database-under-test, the three verbs the schema suites drive it with, and
//! the covering a publishable fixture owes its rows.
//!
//! Everything here is shared because it has **no per-suite content at all**: a
//! migrated `SQLite` database, a statement that must land, a statement whose
//! result is read back, and the coverage `inst-wc-required` demands of any
//! fixture that publishes. Each of the schema suites once carried its own copy
//! of the first three, and the copies had already stopped agreeing.
//!
//! [`author_covering_intention`] is the publishable-draft covering (explicit
//! `AtPublish` create). [`schedule_coverage_window`] remains for tests that
//! specifically exercise historical or committed live `pricing_price_window`
//! rows.
//!
//! What is deliberately **not** here is `must_be_rejected`. Every suite asserts
//! that a refusal is *the one under test* — a raw "some error happened" would
//! pass with the guard it means to prove switched off — and the fragment that
//! makes that assertion sharp is different in each: the table name for the two
//! trigger suites, the constraint name for the CHECK suite. Hoisting them into
//! one helper would mean taking the weakest of the three, which is how a suite
//! ends up green against a schema that no longer holds.
//!
//! The chain is applied **sorted by migration name**, which is the order the
//! `Migrator` itself defines and which `tests/module_test.rs` pins: a table's
//! foreign key and every trigger that reads a parent depend on that parent
//! existing, so a suite that applied the chain in declaration order would fail
//! for a reason having nothing to do with what it is testing.

#![allow(
    dead_code,
    reason = "each test binary compiles the whole module and uses part of it"
)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner,
};
use bss_pricing::domain::instant::utc_ymd_hms;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::draft_window::{self, DraftWindowCommand};
use bss_pricing::infra::storage::entity::{plan, price, region_taxonomy, window_guard};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::window_repo::{NewWindow, WindowRecord, schedule};
use time::OffsetDateTime;

/// An in-memory `SQLite` database carrying the whole migration chain.
pub async fn migrated_db() -> DatabaseConnection {
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let mut chain: Vec<Box<dyn MigrationTrait>> = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }
    conn
}

/// Run one statement and hand back whatever the driver said about it.
///
/// # Errors
/// Whatever the driver refused with — which is the point: the schema suites are
/// about which rejections the database produces.
pub async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        sql.to_owned(),
    ))
    .await
    .map(|_| ())
}

/// Run one statement that must land.
///
/// # Panics
/// When the statement is refused. Every suite here proves a *whitelist* rather
/// than a blanket ban, so the moves that are supposed to work are as
/// load-bearing as the ones that are not.
pub async fn must_succeed(conn: &DatabaseConnection, sql: &str) {
    exec(conn, sql)
        .await
        .unwrap_or_else(|e| panic!("statement must succeed: {sql}\n{e}"));
}

/// Read one value back, from a query that aliases it `v`.
///
/// # Panics
/// When the query fails, returns no row, or the value is not text. Assertions
/// on what actually landed are how these suites tell "the guard refused" from
/// "the guard refused and the statement took effect anyway".
pub async fn scalar(conn: &DatabaseConnection, sql: &str) -> String {
    let row = conn
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query")
        .expect("one row");
    row.try_get::<String>("", "v").expect("read value")
}

/// Insert the per-plan lock row `PriceRepo::create_draft` acquires.
///
/// Idempotent on `(tenant_id, plan_id)` so a fixture that authors two rows of
/// the same plan can call it twice. This is lock identity, not coverage: it
/// does not schedule a window and does not capture a baseline.
pub async fn seed_window_guard(provider: &DBProvider<DbError>, tenant_id: Uuid, plan_id: Uuid) {
    let conn = provider.conn().expect("conn");
    let row = window_guard::ActiveModel {
        tenant_id: Set(tenant_id),
        plan_id: Set(plan_id),
        serial: Set(0),
    };
    let on_conflict =
        OnConflict::columns([window_guard::Column::TenantId, window_guard::Column::PlanId])
            .do_nothing()
            .to_owned();
    match window_guard::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&AccessScope::for_tenant(tenant_id), &row)
        .expect("scope the seeded window guard")
        .on_conflict_raw(on_conflict)
        .exec(&conn)
        .await
    {
        Ok(_) | Err(toolkit_db::secure::ScopeError::Db(DbErr::RecordNotInserted)) => {}
        Err(e) => panic!("seed the lock row create_draft acquires: {e}"),
    }
}

// ---------------------------------------------------------------------------
// The coverage every publishable fixture owes (`inst-wc-required`)
// ---------------------------------------------------------------------------

/// Inclusive start of the fixture coverage window: **2099-08-04T00:00:00Z**.
///
/// The window this schedules must be legitimately `scheduled` — created, not yet
/// in force — because a fixture in a state the system would immediately leave is a
/// fixture that asserts about a world the system does not have. **The year is what
/// makes that a fact instead of an expiry date.** It was `2026-08-04` and the
/// argument for it was that this is one day after the instant the publish suites
/// author their plans at (`2026-08-03`) — true inside the suites' own clock, and
/// false against the real one from the very afternoon it was written: the start had
/// already passed, and four weeks later the whole interval would have been in the
/// past, where a sweep against a real clock reads `expired`, `COVERING_STATES`
/// excludes it, and every publishable fixture in the crate fails
/// `inst-wc-required` on a Tuesday somebody has to bisect. `sqlite_window_repo.rs`
/// is the same lesson already learned: it was dated `2026-09` and would have begun
/// failing on `2026-09-15`, which is why its own scale says the year is
/// load-bearing.
///
/// Still a **fixed** date and not an offset from `now`, which was the right call
/// and is unchanged: a fixture computed from the clock asserts something different
/// every day it runs. What changes is that "the future" is now a fact rather than a
/// date that ages.
pub const COVERAGE_FROM_UTC: (i32, u32, u32) = (2099, 8, 4);

/// Exclusive end of the fixture coverage window: **2099-09-01T00:00:00Z**.
///
/// It stops exactly where the window suites' own instant scale starts —
/// `sqlite_read_model.rs`'s `window_at(0)` is `2099-09-01T00:00:00Z` — so **every**
/// window a suite builds on that scale is adjacent to this one or later, and never
/// overlaps it. `window_repo::schedule` refuses an overlap on the key
/// (`WINDOW_OVERLAP`), so an open-ended fixture window would have made the window
/// suites unable to schedule anything at all; adjacency is legal (§9, half-open
/// `[from, to)`) and is the shape a supersession and a cutover both produce.
///
/// **The residue, named rather than left to be hit.** Because this interval sits
/// *before* the window scale, a sweep run at any instant on that scale finds this
/// window due to activate too — so a test that counts activations sees one more
/// than its own case produced. Two tests in `sqlite_read_model.rs` §8/§9 cancel
/// this window for that reason and say so. Dating it *after* the scale instead
/// was rejected: it would then be the far end of every key's coverage and would
/// swamp the `coverageEnd` assertions that are the whole point of those tests.
pub const COVERAGE_TO_UTC: (i32, u32, u32) = (2099, 9, 1);

/// The fixture window's inclusive start, as an instant.
///
/// Public because a suite asserting what a **frozen version** says about this window
/// has to name an instant inside it, and rebuilding the date from
/// [`COVERAGE_FROM_UTC`] at the call site would be a second spelling of the one fact
/// this module exists to keep single.
#[must_use]
pub fn coverage_from() -> OffsetDateTime {
    midnight(COVERAGE_FROM_UTC)
}

/// The end of that coverage, as an instant.
///
/// [`coverage_from`]'s sibling, and it exists for the sharper half of that reason: a
/// suite naming an instant **outside** the fixture's coverage — a dormant key, a
/// changeover past the end — would otherwise rebuild the date from
/// [`COVERAGE_TO_UTC`] at the call site, which is a second spelling of the one fact
/// this module keeps single.
#[must_use]
pub fn coverage_to() -> OffsetDateTime {
    midnight(COVERAGE_TO_UTC)
}

/// The two instants above, as instants.
fn midnight(date: (i32, u32, u32)) -> OffsetDateTime {
    utc_ymd_hms(date.0, date.1, date.2, 0, 0, 0)
}

/// The window id this fixture gives `price_id`'s coverage.
///
/// Derived from the row rather than passed in, so a seed cannot mint two windows
/// on one row by accident and two seeds in one test cannot collide on an id. The
/// salt keeps it out of the `Uuid::from_u128(small)` space every fixture in this
/// crate hand-picks ids from.
#[must_use]
pub fn coverage_window_id(price_id: Uuid) -> Uuid {
    /// Arbitrary and fixed; only its distance from the hand-picked ids matters.
    const SALT: u128 = 0x_c0de_face_0000_0000_0000_0000_0000_0000;
    Uuid::from_u128(price_id.as_u128() ^ SALT)
}

/// Author an explicit covering create on an open draft (`inst-wc-required`).
///
/// Publishable never-published fixtures use this instead of
/// [`schedule_coverage_window`]: a live `pricing_price_window` on a draft makes
/// assemble refuse `WindowBaselineChanged`, and compose ignores live seed
/// windows. Prefer [`DraftStart::AtPublish`] with an open end.
///
/// The operation id is [`coverage_window_id`] so suites that later cancel or
/// shorten the materialized covering can name it.
///
/// # Panics
/// When the command is refused. That is the fixture being wrong about the world.
#[allow(clippy::too_many_arguments)]
pub async fn author_covering_intention(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    revision: u64,
    expected: RowVersion,
    price_id: Uuid,
    start: DraftStart,
    effective_to: Option<OffsetDateTime>,
    stamp: AuditStamp,
) -> RowVersion {
    let window_id = coverage_window_id(price_id);
    let next = draft_window::apply_command(
        runner,
        scope,
        &DraftWindowOwner {
            tenant_id,
            plan_id: plan_id.get(),
            plan_revision: revision,
        },
        expected.get(),
        DraftWindowCommand::Put(DraftWindowEntry {
            operation_id: window_id,
            action: DraftWindowAction::Create {
                window_id,
                price_id,
                start,
                effective_to,
            },
            reason_code: "fixtureCoverage".to_owned(),
        }),
        stamp,
    )
    .await
    .unwrap_or_else(|e| panic!("author covering intention for price row {price_id}: {e}"));
    RowVersion::new(next)
}

/// Schedule a **live** `pricing_price_window` for tests that exercise
/// historical or already-committed coverage — not for publishable-draft setup.
///
/// Never-published plans that must pass `inst-wc-required` should call
/// [`author_covering_intention`] instead. A live seed on a draft is ignored by
/// compose and makes captured-baseline drift refuse the publish.
///
/// **`window_repo::schedule` and not `POST …/prices/{priceId}/windows`, for
/// isolation and speed.** The route is mounted and works — `rest_windows.rs` drives
/// it — so this is a choice rather than a workaround. Two grounds: a mutation
/// through the service resolves the plan's *current* revision, and historical
/// fixtures often need a window before any draft context exists; and routing
/// every live seed through the schedule path would make one defect in it redden
/// four suites at once.
///
/// The window is `scheduled` — the only state `window_repo::schedule` creates,
/// §4's initial one — over `[COVERAGE_FROM_UTC, COVERAGE_TO_UTC)`. Both instants
/// are **fixed dates and not offsets from `now`**.
///
/// # Panics
/// When the schedule is refused — an overlap on the key, a row that is not there,
/// or a scope that cannot see it. Each of those is the fixture being wrong about
/// the world rather than the store misbehaving.
pub async fn schedule_coverage_window(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
    stamp: AuditStamp,
) -> WindowRecord {
    schedule(
        runner,
        scope,
        NewWindow {
            window_id: coverage_window_id(price_id),
            tenant_id,
            price_id,
            effective_from: midnight(COVERAGE_FROM_UTC),
            effective_to: Some(midnight(COVERAGE_TO_UTC)),
            reason_code: "fixtureCoverage".to_owned(),
        },
        stamp,
    )
    .await
    .unwrap_or_else(|e| panic!("schedule the coverage window of price row {price_id}: {e}"))
}

/// Open-ended live covering from [`coverage_from`], for committed-state suites
/// whose assemble path now judges `inst-wc-required` over live windows.
///
/// Finite [`schedule_coverage_window`] is the historical `[FROM, TO)` fixture
/// `rest_windows` posts adjacent to. Repricing apply runs publish aggregate
/// rules; a finite tail is `WINDOW_TRAILING_VOID`.
pub async fn schedule_open_ended_coverage(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    price_id: Uuid,
    stamp: AuditStamp,
) -> WindowRecord {
    schedule(
        runner,
        scope,
        NewWindow {
            window_id: coverage_window_id(price_id),
            tenant_id,
            price_id,
            effective_from: midnight(COVERAGE_FROM_UTC),
            effective_to: None,
            reason_code: "fixtureCoverage".to_owned(),
        },
        stamp,
    )
    .await
    .unwrap_or_else(|e| panic!("schedule open-ended covering of price row {price_id}: {e}"))
}

/// Declare the region universe every publishing fixture's rows sell in.
///
/// **Required since `inst-tx-region` was registered in the Foundation rule set.**
/// C2 is fail-closed — a tenant whose region taxonomy declares nothing publishes
/// nothing — so a fixture that publishes a plan has to declare the region its
/// rows carry, exactly as a real operator would through
/// `PUT /config/vocabularies/region`.
///
/// Before the rule was registered these fixtures published rows in regions no
/// tenant had ever declared, which is a world the system is not supposed to be
/// able to reach. That they were green is not evidence the rule was satisfied;
/// it is evidence nothing was asking.
///
/// The set is the union of the spellings the suites use — `region` is
/// case-sensitive and `eu` and `EU` are two different values on the axis.
pub async fn declare_fixture_regions(provider: &DBProvider<DbError>, tenant_id: Uuid) {
    let conn = provider.conn().expect("conn");
    for value in ["eu", "EU", "us", "US", "DE", "us-east"] {
        let row = region_taxonomy::ActiveModel {
            tenant_id: Set(tenant_id),
            value: Set(value.to_owned()),
            display_name: Set(format!("fixture region {value}")),
            state: Set("active".to_owned()),
            // **Both D-01 markers declared**, and not for tidiness:
            // `inst-td-policy` is registered in the Foundation set, its category
            // arm is unconditional (D-154) and C4's rate arm is fail-closed, so a
            // fixture region declaring neither would fail every publish in the
            // crate on a rule none of those suites is about. A real operator
            // declares them in the same `PUT` that declares the region.
            tax_category: Set(Some("standard".to_owned())),
            tax_rate_present: Set(true),
        };
        region_taxonomy::Entity::insert(row.clone())
            .secure()
            .scope_with_model(&AccessScope::allow_all(), &row)
            .expect("scope")
            .exec(&conn)
            .await
            .expect("declare a fixture region");
    }
}

/// Retire one declared fixture region, past the repository's guard.
///
/// Direct because the guard is not what these cases are about: they need a value
/// that *is* `retired`, and reaching it through `PUT /config/vocabularies/region`
/// would make an unrelated refusal there look like the failure under test.
pub async fn retire_fixture_region(provider: &DBProvider<DbError>, tenant_id: Uuid, value: &str) {
    let conn = provider.conn().expect("conn");
    let affected = region_taxonomy::Entity::update_many()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .col_expr(region_taxonomy::Column::State, Expr::value("retired"))
        .filter(
            Condition::all()
                .add(region_taxonomy::Column::TenantId.eq(tenant_id))
                .add(region_taxonomy::Column::Value.eq(value)),
        )
        .exec(&conn)
        .await
        .expect("retire the fixture region");
    assert_eq!(affected.rows_affected, 1, "the fixture region must exist");
}

/// Move a price row to `published` directly, past every door.
///
/// **Fabricated on purpose, and the reason is `sqlite_price_repo::flip_state`'s.**
/// A suite whose subject is something *else* must not depend on the publish
/// engine's four preconditions to put a row in the state its fixture needs — the
/// engine has its own suites, and borrowing it here would make an unrelated
/// failure read as this file's. The flip is legal because the append-only trigger
/// **whitelists** `draft -> published`, not because the trigger is inert on a
/// draft: it has a draft branch on both backends (D-153).
pub async fn publish_row_directly(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) {
    let conn = provider.conn().expect("conn");
    let result = price::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            price::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .exec(&conn)
        .await
        .expect("publish the seeded row");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

/// Move a published price row to `superseded` directly, past the engine.
///
/// The live `pricing_price_window` on the row is left in place: activation still
/// expires superseded covering, and draft baseline capture must omit it.
pub async fn supersede_row_directly(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) {
    let conn = provider.conn().expect("conn");
    let result = price::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            price::Column::LifecycleState,
            Expr::value(LifecycleState::Superseded.as_str()),
        )
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .exec(&conn)
        .await
        .expect("supersede the seeded row");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

/// Move a plan revision to `superseded` directly, past the engine.
///
/// [`publish_plan_directly`] is a blunt UPDATE and does not demote the revision it
/// replaces, so a fixture that wants a plan sitting **past** revision 0 has to
/// demote 0 itself: `uq_pricing_plan_current` is partial over
/// `('published','retired')` and admits one row per plan.
pub async fn supersede_plan_directly(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    plan_id: bss_pricing::domain::scope_key::PlanId,
    revision: u64,
) {
    let conn = provider.conn().expect("conn");
    let result = plan::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            plan::Column::LifecycleState,
            Expr::value(LifecycleState::Superseded.as_str()),
        )
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::Revision.eq(i64::try_from(revision).expect("a small revision"))),
        )
        .exec(&conn)
        .await
        .expect("supersede the seeded plan revision");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

/// Move a plan revision to `published` directly, past the engine.
///
/// [`publish_row_directly`]'s reason exactly. It exists so a fixture can make
/// `plan_repo::load_current` answer with a revision — which is what D-212's
/// resolver reads to decide *which* revision of a referencing bundle counts.
pub async fn publish_plan_directly(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    plan_id: bss_pricing::domain::scope_key::PlanId,
    revision: u64,
) {
    let conn = provider.conn().expect("conn");
    let result = plan::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            plan::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::Revision.eq(i64::try_from(revision).expect("a small revision"))),
        )
        .exec(&conn)
        .await
        .expect("publish the seeded plan revision");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

mod catalog;
#[allow(
    unused_imports,
    reason = "each integration binary uses a different part of common"
)]
pub use catalog::FixtureCatalog;

// ---------------------------------------------------------------------------
// The charge-line graph a seeded price row hangs off.
// ---------------------------------------------------------------------------

/// The logical axes and shared structure a seeded `pricing_price` row needs
/// behind it.
///
/// **Why a fixture needs this at all.** A price row no longer carries its own
/// scope key or its own structure: the eight logical axes live on
/// `pricing_charge_line`, the shared calculation on
/// `pricing_charge_line_version`, and currency/region on
/// `pricing_market_price`. A suite that inserts a bare `price::ActiveModel`
/// writes three dangling foreign keys, so every fixture that seeds a row by
/// hand seeds its graph first — through this one helper, because the copies
/// would stop agreeing exactly as the three schema-suite copies of
/// [`migrated_db`] once did.
///
/// The defaults are the same ones the suites were already spelling: the base
/// overlay, the ungrandfathered `all_subscriptions` class, an empty dimension
/// and a `published` lifecycle.
pub struct ChargeGraphSeed {
    pub tenant_id: Uuid,
    pub plan_id: Uuid,
    pub plan_revision: i64,
    pub phase: Uuid,
    pub sku_id: Uuid,
    pub price_overlay: String,
    pub price_eligibility: String,
    pub charge_kind: String,
    pub cohort: String,
    pub dimension_key: String,
    pub currency: String,
    pub region: String,
    pub lifecycle_state: String,
    pub model_kind: Option<String>,
    pub meter: Option<String>,
    pub invoice_line_template: Option<String>,
    pub gl_code_ref: Option<String>,
    pub resolved_invoice_line_template: Option<String>,
    pub resolved_gl_code: Option<String>,
    pub created_by: Uuid,
    pub created_at_utc: OffsetDateTime,
}

impl Default for ChargeGraphSeed {
    fn default() -> Self {
        Self {
            tenant_id: Uuid::nil(),
            plan_id: Uuid::nil(),
            plan_revision: 1,
            phase: Uuid::nil(),
            sku_id: Uuid::nil(),
            price_overlay: "base".to_owned(),
            price_eligibility: "all_subscriptions".to_owned(),
            charge_kind: "recurring".to_owned(),
            cohort: "none".to_owned(),
            dimension_key: String::new(),
            currency: "EUR".to_owned(),
            region: "eu".to_owned(),
            lifecycle_state: "published".to_owned(),
            model_kind: None,
            meter: None,
            invoice_line_template: None,
            gl_code_ref: None,
            resolved_invoice_line_template: None,
            resolved_gl_code: None,
            created_by: Uuid::nil(),
            created_at_utc: utc_ymd_hms(2026, 1, 1, 0, 0, 0),
        }
    }
}

/// The three immutable references a seeded price row fills in.
#[derive(Clone, Copy, Debug)]
#[allow(
    clippy::struct_field_names,
    reason = "every field is an id because the struct is nothing but references"
)]
pub struct SeededGraph {
    pub charge_line_id: Uuid,
    pub line_version_id: Uuid,
    pub market_price_id: Uuid,
}

/// Derive a stable id from a logical key, so re-seeding the same key twice
/// addresses the same row rather than colliding on its unique constraint.
fn seeded_id(namespace: u128, parts: &[&[u8]]) -> Uuid {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(part);
        bytes.push(0xff);
    }
    Uuid::new_v5(&Uuid::from_u128(namespace), &bytes)
}

/// Seed (or reuse) the charge line, its version and its market.
///
/// Find-or-insert on each of the three, keyed the way the table's own unique
/// constraint is, so a suite may call this once per price row of a line without
/// minting a second line, a second version of one revision, or a second market.
pub async fn seed_charge_graph(
    runner: &impl DBRunner,
    scope: &AccessScope,
    seed: &ChargeGraphSeed,
) -> SeededGraph {
    use bss_pricing::infra::storage::entity::{charge_line, charge_line_version, market_price};

    let charge_line_id = seeded_id(
        0x5f01,
        &[
            seed.tenant_id.as_bytes(),
            seed.plan_id.as_bytes(),
            seed.phase.as_bytes(),
            seed.price_overlay.as_bytes(),
            seed.price_eligibility.as_bytes(),
            seed.charge_kind.as_bytes(),
            seed.cohort.as_bytes(),
            seed.sku_id.as_bytes(),
            seed.dimension_key.as_bytes(),
        ],
    );
    // **Found by its logical scope, not by the id this helper would mint.** The
    // repository derives `charge_line_id` with its own namespace, so a fixture
    // that seeds beside a row the repository already wrote would look for an id
    // that is not there, insert, and collide on
    // `uq_pricing_charge_line_logical_scope` instead. The scope is what "the same
    // line" means, so the scope is what the lookup asks about.
    let present = charge_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line::Column::TenantId.eq(seed.tenant_id))
                .add(charge_line::Column::PlanId.eq(seed.plan_id))
                .add(charge_line::Column::Phase.eq(seed.phase))
                .add(charge_line::Column::SkuId.eq(seed.sku_id))
                .add(charge_line::Column::PriceOverlay.eq(seed.price_overlay.clone()))
                .add(charge_line::Column::PriceEligibility.eq(seed.price_eligibility.clone()))
                .add(charge_line::Column::ChargeKind.eq(seed.charge_kind.clone()))
                .add(charge_line::Column::Cohort.eq(seed.cohort.clone()))
                .add(charge_line::Column::DimensionKey.eq(seed.dimension_key.clone())),
        )
        .one(runner)
        .await
        .expect("read back the seeded charge line");
    let charge_line_id = present
        .as_ref()
        .map_or(charge_line_id, |row| row.charge_line_id);
    if present.is_none() {
        let line = charge_line::ActiveModel {
            tenant_id: Set(seed.tenant_id),
            charge_line_id: Set(charge_line_id),
            plan_id: Set(seed.plan_id),
            phase: Set(seed.phase),
            price_overlay: Set(seed.price_overlay.clone()),
            price_eligibility: Set(seed.price_eligibility.clone()),
            charge_kind: Set(seed.charge_kind.clone()),
            cohort: Set(seed.cohort.clone()),
            sku_id: Set(seed.sku_id),
            dimension_key: Set(seed.dimension_key.clone()),
        };
        charge_line::Entity::insert(line.clone())
            .secure()
            .scope_with_model(scope, &line)
            .expect("scope the seeded charge line")
            .exec(runner)
            .await
            .expect("seed the charge line");
    }

    let line_version_id = seeded_id(
        0x5f02,
        &[charge_line_id.as_bytes(), &seed.plan_revision.to_be_bytes()],
    );
    let present = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(seed.tenant_id))
                .add(charge_line_version::Column::ChargeLineId.eq(charge_line_id))
                .add(charge_line_version::Column::PlanRevision.eq(seed.plan_revision)),
        )
        .one(runner)
        .await
        .expect("read back the seeded line version");
    let line_version_id = present
        .as_ref()
        .map_or(line_version_id, |row| row.line_version_id);
    if present.is_none() {
        let version = charge_line_version::ActiveModel {
            tenant_id: Set(seed.tenant_id),
            line_version_id: Set(line_version_id),
            charge_line_id: Set(charge_line_id),
            plan_revision: Set(seed.plan_revision),
            lifecycle_state: Set(seed.lifecycle_state.clone()),
            model_kind: Set(seed.model_kind.clone()),
            meter: Set(seed.meter.clone()),
            invoice_line_template: Set(seed.invoice_line_template.clone()),
            gl_code_ref: Set(seed.gl_code_ref.clone()),
            resolved_invoice_line_template: Set(seed.resolved_invoice_line_template.clone()),
            resolved_gl_code: Set(seed.resolved_gl_code.clone()),
            created_by: Set(seed.created_by),
            created_at_utc: Set(seed.created_at_utc),
            row_version: Set(0),
            ..Default::default()
        };
        charge_line_version::Entity::insert(version.clone())
            .secure()
            .scope_with_model(scope, &version)
            .expect("scope the seeded line version")
            .exec(runner)
            .await
            .expect("seed the line version");
    }

    let market_price_id = seeded_id(
        0x5f03,
        &[
            charge_line_id.as_bytes(),
            seed.currency.as_bytes(),
            seed.region.as_bytes(),
        ],
    );
    let present = market_price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(market_price::Column::TenantId.eq(seed.tenant_id))
                .add(market_price::Column::ChargeLineId.eq(charge_line_id))
                .add(market_price::Column::Currency.eq(seed.currency.clone()))
                .add(market_price::Column::Region.eq(seed.region.clone())),
        )
        .one(runner)
        .await
        .expect("read back the seeded market");
    let market_price_id = present
        .as_ref()
        .map_or(market_price_id, |row| row.market_price_id);
    if present.is_none() {
        let market = market_price::ActiveModel {
            tenant_id: Set(seed.tenant_id),
            market_price_id: Set(market_price_id),
            charge_line_id: Set(charge_line_id),
            currency: Set(seed.currency.clone()),
            region: Set(seed.region.clone()),
        };
        market_price::Entity::insert(market.clone())
            .secure()
            .scope_with_model(scope, &market)
            .expect("scope the seeded market")
            .exec(runner)
            .await
            .expect("seed the market");
    }

    SeededGraph {
        charge_line_id,
        line_version_id,
        market_price_id,
    }
}

// ---------------------------------------------------------------------------
// The same graph, for the suites that write raw SQL.
// ---------------------------------------------------------------------------

/// A charge-line graph expressed as SQL literals.
///
/// [`seed_charge_graph`]'s sibling for the schema suites, which write their rows
/// by hand precisely so a repository precheck cannot answer for a missing
/// constraint. They still need the three parent rows a `pricing_price` row
/// references, and they need them spelled the same way — a second copy of this
/// INSERT in each suite is how the copies stopped agreeing before.
pub struct SqlGraphSeed<'a> {
    pub tenant_id: &'a str,
    pub plan_id: &'a str,
    pub phase: &'a str,
    pub sku_id: &'a str,
    pub charge_kind: &'a str,
    pub price_eligibility: &'a str,
    pub cohort: &'a str,
    pub dimension_key: &'a str,
    pub currency: &'a str,
    pub region: &'a str,
    pub model_kind: Option<&'a str>,
    pub lifecycle_state: &'a str,
    pub created_by: &'a str,
    pub created_at_utc: &'a str,
    pub plan_revision: i64,
}

impl<'a> SqlGraphSeed<'a> {
    /// The defaults the schema suites were already spelling.
    #[must_use]
    pub fn new(tenant_id: &'a str, plan_id: &'a str, phase: &'a str, sku_id: &'a str) -> Self {
        Self {
            tenant_id,
            plan_id,
            phase,
            sku_id,
            charge_kind: "recurring",
            price_eligibility: "all_subscriptions",
            cohort: "none",
            dimension_key: "",
            currency: "USD",
            region: "EU",
            model_kind: Some("flat"),
            lifecycle_state: "published",
            created_by: tenant_id,
            created_at_utc: "2026-08-02 10:00:00 +00:00",
            plan_revision: 0,
        }
    }
}

/// The three ids a seeded price row references.
#[derive(Clone, Debug)]
#[allow(
    clippy::struct_field_names,
    reason = "every field is an id because the struct is nothing but references"
)]
pub struct SqlGraphIds {
    pub charge_line_id: String,
    pub line_version_id: String,
    pub market_price_id: String,
}

/// Seed (or reuse) the line, version and market, in SQL.
///
/// `INSERT OR IGNORE` on each of the three, because a suite seeds several price
/// rows of one line and the second call must address the first call's rows
/// rather than collide with them. The ids are derived from the logical key, so
/// "the same key" and "the same row" are the same statement.
///
/// # Panics
/// When a statement the schema should accept is refused.
pub async fn seed_charge_graph_sql(
    conn: &DatabaseConnection,
    seed: &SqlGraphSeed<'_>,
) -> SqlGraphIds {
    let charge_line_id = seeded_id(
        0x5f01,
        &[
            seed.tenant_id.as_bytes(),
            seed.plan_id.as_bytes(),
            seed.phase.as_bytes(),
            seed.price_eligibility.as_bytes(),
            seed.charge_kind.as_bytes(),
            seed.cohort.as_bytes(),
            seed.sku_id.as_bytes(),
            seed.dimension_key.as_bytes(),
        ],
    )
    .to_string();
    let line_version_id = seeded_id(
        0x5f02,
        &[charge_line_id.as_bytes(), &seed.plan_revision.to_be_bytes()],
    )
    .to_string();
    let market_price_id = seeded_id(
        0x5f03,
        &[
            charge_line_id.as_bytes(),
            seed.currency.as_bytes(),
            seed.region.as_bytes(),
        ],
    )
    .to_string();

    must_succeed(
        conn,
        &format!(
            "INSERT OR IGNORE INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, \
             phase, price_eligibility, charge_kind, cohort, sku_id, dimension_key) VALUES \
             ('{}','{charge_line_id}','{}','{}','{}','{}','{}','{}','{}')",
            seed.tenant_id,
            seed.plan_id,
            seed.phase,
            seed.price_eligibility,
            seed.charge_kind,
            seed.cohort,
            seed.sku_id,
            seed.dimension_key,
        ),
    )
    .await;

    let model_kind = seed
        .model_kind
        .map_or_else(|| "NULL".to_owned(), |kind| format!("'{kind}'"));
    must_succeed(
        conn,
        &format!(
            "INSERT OR IGNORE INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{}','{line_version_id}','{charge_line_id}',{},'{}',{model_kind},'{}','{}',0)",
            seed.tenant_id,
            seed.plan_revision,
            seed.lifecycle_state,
            seed.created_by,
            seed.created_at_utc,
        ),
    )
    .await;

    must_succeed(
        conn,
        &format!(
            "INSERT OR IGNORE INTO pricing_market_price (tenant_id, market_price_id, \
             charge_line_id, currency, region) VALUES \
             ('{}','{market_price_id}','{charge_line_id}','{}','{}')",
            seed.tenant_id, seed.currency, seed.region,
        ),
    )
    .await;

    SqlGraphIds {
        charge_line_id,
        line_version_id,
        market_price_id,
    }
}

/// The market id [`seed_charge_graph_sql`] derives for one logical key.
///
/// Exposed because a window row has to name its price's market without
/// re-seeding the graph: non-overlap is judged per market and the table's
/// foreign key is the compound `(tenant_id, price_id, market_price_id)`, so a
/// fixture that writes windows by hand needs the id the seeder minted.
#[must_use]
pub fn sql_market_id(
    tenant_id: &str,
    plan_id: &str,
    phase: &str,
    sku_id: &str,
    charge_kind: &str,
    currency: &str,
    region: &str,
) -> String {
    let charge_line_id = seeded_id(
        0x5f01,
        &[
            tenant_id.as_bytes(),
            plan_id.as_bytes(),
            phase.as_bytes(),
            b"all_subscriptions",
            charge_kind.as_bytes(),
            b"none",
            sku_id.as_bytes(),
            b"",
        ],
    )
    .to_string();
    seeded_id(
        0x5f03,
        &[
            charge_line_id.as_bytes(),
            currency.as_bytes(),
            region.as_bytes(),
        ],
    )
    .to_string()
}

/// The line-version id [`seed_charge_graph_sql`] derives for one logical key and
/// revision.
///
/// [`sql_market_id`]'s sibling, for the cases whose subject is shared content
/// rather than a market.
#[must_use]
pub fn sql_line_version_id(
    tenant_id: &str,
    plan_id: &str,
    phase: &str,
    sku_id: &str,
    charge_kind: &str,
    plan_revision: i64,
) -> String {
    let charge_line_id = seeded_id(
        0x5f01,
        &[
            tenant_id.as_bytes(),
            plan_id.as_bytes(),
            phase.as_bytes(),
            b"all_subscriptions",
            charge_kind.as_bytes(),
            b"none",
            sku_id.as_bytes(),
            b"",
        ],
    )
    .to_string();
    seeded_id(
        0x5f02,
        &[charge_line_id.as_bytes(), &plan_revision.to_be_bytes()],
    )
    .to_string()
}
