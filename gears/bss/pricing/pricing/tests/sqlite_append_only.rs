//! The append-only column whitelist on `pricing_price`, proven against a real
//! database.
//!
//! Postgres carries the whitelist as one PL/pgSQL trigger; `SQLite` mirrors it
//! as five `RAISE(ABORT, ...)` triggers (see the migration's module doc), so the
//! guard is exercisable without Docker and this suite does not need a
//! testcontainers test to know the rule holds.
//!
//! One case per branch of the whitelist: a forbidden price mutation, a
//! forbidden lifecycle transition on the published plane **and on the draft one
//! (D-153)**, a forbidden loosening of `grandfather_until`,
//! a forbidden DELETE, a forbidden bump of `row_version`, and a forbidden
//! re-size of a `package` block — plus the moves that are *supposed* to work,
//! so the test proves a whitelist rather than a blanket ban. Without the guard
//! an ad-hoc UPDATE would silently change a frozen `CatalogVersion`'s content
//! at the next warm re-drive, because the projector reads truth rows.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed, scalar};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const PHASE: &str = "33333333-3333-3333-3333-333333333333";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const PUBLISHED: &str = "55555555-5555-5555-5555-555555555555";
const DRAFT: &str = "66666666-6666-6666-6666-666666666666";

/// Reject, **and** for the stated reason.
///
/// The fragment is not decoration. `pricing_price` carries five whitelist
/// triggers, twenty `CHECK` constraints and two partial `UNIQUE` indexes, and
/// every one of those names contains the string `pricing_price` — as does the
/// column list `SQLite` reports for a unique violation. A test that accepted
/// any error naming the table would therefore pass with the trigger it means to
/// prove switched off, refused instead by a constraint it never intended to
/// trip.
async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the append-only guard must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("pricing_price"),
        "the rejection must name the guard it came from, got: {message}"
    );
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// Insert one published `all_subscriptions` row and one draft row on the same
/// plan (different charge kinds, so the scope-key unique index is satisfied).
async fn seed(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, amount_minor, model_kind, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{PUBLISHED}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'recurring', 1000, 'flat', 'published', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, amount_minor, model_kind, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{DRAFT}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'one_time', 500, 'flat', 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_published_price_row_is_immutable_in_content() {
    let conn = migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET amount_minor = 1 WHERE price_id = '{PUBLISHED}'"),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET currency = 'EUR' WHERE price_id = '{PUBLISHED}'"),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET model_kind = 'volume' WHERE price_id = '{PUBLISHED}'"),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;
    // The two tax columns (`T-18`, `m20260802_000040`), and they are here on the
    // **fast** tier deliberately. Postgres has the exhaustive thirty-four-column
    // loop, but it is `#[ignore]`d behind Docker — and D-236 is the record of what
    // that costs: a premise living on one tier only means a run without Docker
    // reports a clean change through a guard that stopped guarding.
    //
    // `tax_category_ref` is the sharper of the two: it is authored draft content,
    // D-48 makes it one of the five descriptor elements Billing countersigns, and
    // since the pin moved to `v5` it is inside the approval content hash — so a
    // published row whose category moved would diverge from the pin that approved
    // it.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET tax_category_ref = 'reduced' WHERE price_id = '{PUBLISHED}'"
        ),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET resolved_tax_category = 'standard' \
             WHERE price_id = '{PUBLISHED}'"
        ),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;

    let amount = scalar(
        &conn,
        &format!("SELECT CAST(amount_minor AS TEXT) AS v FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    assert_eq!(amount, "1000", "no rejected UPDATE may have landed");
}

#[tokio::test]
async fn only_the_sanctioned_lifecycle_transition_is_permitted() {
    let conn = migrated_db().await;
    seed(&conn).await;

    // `published -> retired` is a plan-revision flip, not a price-row one.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'retired' WHERE price_id = '{PUBLISHED}'"
        ),
        "lifecycle_state transition is not sanctioned",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'draft' WHERE price_id = '{PUBLISHED}'"
        ),
        "lifecycle_state transition is not sanctioned",
    )
    .await;

    // `published -> superseded` is the one the state machine sanctions.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'superseded' WHERE price_id = '{PUBLISHED}'"
        ),
    )
    .await;
    let state = scalar(
        &conn,
        &format!("SELECT lifecycle_state AS v FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    assert_eq!(state, "superseded");
}

#[tokio::test]
async fn a_draft_row_may_only_go_to_published_d153() {
    // The draft plane's own whitelist. A column whitelist is scoped to
    // *published* rows by construction, so before D-153 this trigger returned
    // early for a draft row and `draft -> superseded` was physically possible.
    // Such a ghost lands outside **both** partial `UNIQUE` predicates — its key
    // reads free on the published plane and on the draft plane — undoing the
    // guarantee D-148 bought, and `inst-ps-nodelete` then makes it undeletable
    // on a key no supersession chain reaches.
    let conn = migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'superseded' WHERE price_id = '{DRAFT}'"
        ),
        "lifecycle_state transition is not sanctioned",
    )
    .await;
    assert_eq!(
        scalar(
            &conn,
            &format!("SELECT lifecycle_state AS v FROM pricing_price WHERE price_id = '{DRAFT}'"),
        )
        .await,
        "draft",
        "the refused transition must not have taken effect"
    );

    // The two edges the machine does sanction from `draft`: staying put while
    // content is edited, and publishing.
    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET amount_minor = 4242 WHERE price_id = '{DRAFT}'"),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'published' WHERE price_id = '{DRAFT}'"
        ),
    )
    .await;
    assert_eq!(
        scalar(
            &conn,
            &format!("SELECT lifecycle_state AS v FROM pricing_price WHERE price_id = '{DRAFT}'"),
        )
        .await,
        "published"
    );
}

#[tokio::test]
async fn grandfather_until_may_only_be_tightened() {
    let conn = migrated_db().await;
    let row = "77777777-7777-7777-7777-777777777777";
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                price_eligibility, charge_kind, cohort, amount_minor, model_kind,
                lifecycle_state, created_by, created_at_utc)
             VALUES ('{row}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'existing_grandfathered', 'recurring', '1780000000000', 900, 'flat',
                'published', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;

    // Setting it when null is a tightening.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2027-01-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
    )
    .await;
    // Moving it earlier is a tightening.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2026-10-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
    )
    .await;
    // Moving it later is a loosening.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2028-01-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
        "grandfather_until may only be tightened, never loosened",
    )
    .await;
    // Clearing it is a loosening too.
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET grandfather_until = NULL WHERE price_id = '{row}'"),
        "grandfather_until may only be tightened, never loosened",
    )
    .await;

    let horizon = scalar(
        &conn,
        &format!("SELECT grandfather_until AS v FROM pricing_price WHERE price_id = '{row}'"),
    )
    .await;
    assert_eq!(horizon, "2026-10-01 00:00:00 +00:00");
}

#[tokio::test]
async fn published_rows_never_delete_and_drafts_stay_mutable() {
    let conn = migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
        "DELETE of a non-draft row is not permitted",
    )
    .await;

    // A never-published draft is freely mutable and deletable — the whitelist
    // guards frozen rows, it does not freeze authoring.
    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET amount_minor = 750 WHERE price_id = '{DRAFT}'"),
    )
    .await;
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_price WHERE price_id = '{DRAFT}'"),
    )
    .await;

    let remaining = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(remaining, "1", "only the published row survives");
}

#[tokio::test]
async fn a_published_rows_entity_tag_is_frozen_with_its_content() {
    let conn = migrated_db().await;
    seed(&conn).await;

    // The tag denotes a representation, and this one cannot change. A tag that
    // moved under frozen content would tell a caller its cached copy is stale
    // when it is not, and fail every correctly-read `If-Match` submit after it.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET row_version = row_version + 1 WHERE price_id = '{PUBLISHED}'"
        ),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;

    let version = scalar(
        &conn,
        &format!(
            "SELECT CAST(row_version AS TEXT) AS v FROM pricing_price WHERE price_id = '{PUBLISHED}'"
        ),
    )
    .await;
    assert_eq!(version, "0", "the rejected bump may not have landed");
}

#[tokio::test]
async fn a_published_rows_package_block_size_is_frozen() {
    let conn = migrated_db().await;
    let row = "88888888-8888-8888-8888-888888888888";
    // A published `package` row, so the kind CHECK that ties the package fields
    // to `model_kind` is already satisfied and the only thing that can reject
    // the UPDATE below is the append-only whitelist.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, model_kind, package_size, package_price_minor,
                lifecycle_state, created_by, created_at_utc)
             VALUES ('{row}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'usage', 'package', 100, 5000, 'published', '{ACTOR}',
                '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;

    // `package_size` is a quantity-determining field, not a price lever: block
    // math is non-linear in the window, so re-sizing a block mid-window
    // re-buckets an already-accumulated counter (D-122). It is frozen for the
    // same reason every other model column is.
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET package_size = 200 WHERE price_id = '{row}'"),
        "price, scope, model and entity-tag columns are immutable",
    )
    .await;

    let size = scalar(
        &conn,
        &format!(
            "SELECT CAST(package_size AS TEXT) AS v FROM pricing_price WHERE price_id = '{row}'"
        ),
    )
    .await;
    assert_eq!(size, "100", "the rejected re-size may not have landed");
}

#[tokio::test]
async fn a_draft_rows_entity_tag_advances_with_each_edit() {
    let conn = migrated_db().await;
    seed(&conn).await;

    // The draft plane is where content moves, so it is where the tag moves.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET row_version = row_version + 1 WHERE price_id = '{DRAFT}'"
        ),
    )
    .await;

    let version = scalar(
        &conn,
        &format!(
            "SELECT CAST(row_version AS TEXT) AS v FROM pricing_price WHERE price_id = '{DRAFT}'"
        ),
    )
    .await;
    assert_eq!(version, "1", "the authoring edit advanced the draft's tag");
}
