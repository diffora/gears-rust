//! The two cross-table guards on `pricing_price_tier_band`, plus its per-row
//! constraints, proven against a real database.
//!
//! Neither guard is a row CHECK: **structural exclusivity** reads the
//! `model_kind` of the *line version* the band's price names, and
//! **append-only-with-the-parent** reads the `lifecycle_state` of the *price
//! row*, and a row CHECK may not read another table. Both are therefore
//! triggers, and a trigger that is silently wrong is a trigger that never fires
//! — so each branch gets a case here, alongside the moves that are *supposed* to
//! work, so this proves a rule rather than a blanket ban.
//!
//! **A band is one row, and it is a market's.** Its bounds sit beside the rate
//! that prices them, keyed by the price row, so two markets of one line version
//! each carry a ladder of their own. The model kind stays the line's — which is
//! why the two guards read two different parents.
//!
//! Structural exclusivity has **two ends**: the child-side arms judge a band as
//! it arrives, and the parent-side arm — a trigger on
//! `pricing_charge_line_version` this table's migration installs — refuses a
//! version that still prices bands becoming a kind that carries none. Without it
//! the forbidden pair is reachable by moving the parent instead of the band, and
//! the version is left in it.
//!
//! Postgres carries both as PL/pgSQL functions; `SQLite` mirrors them as
//! fixed-message `RAISE(ABORT, ...)` triggers whose parent lookup is a
//! `WHERE NOT EXISTS` subquery in the body, so the guards are exercisable
//! without Docker.
//!
//! Ordering and contiguity are deliberately absent from this file: they are
//! sequence properties the `TierBandValidator` owns at publish, not database
//! constraints, and a test here would be testing something that does not exist.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed, scalar};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const PHASE: &str = "33333333-3333-3333-3333-333333333333";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
/// The SKU every seeded row prices (D-372). `pricing_price.sku_id` and
/// `pricing_plan.sku_id` are `NOT NULL` in the fresh-install DDL,
/// so a seed names one; the value itself is incidental to these cases.
const SKU: &str = "00000000-0000-0000-0000-000000000005";
/// A `graduated` parent — the kind that may carry bands.
const TIERED: &str = "55555555-5555-5555-5555-555555555555";
/// A `flat` parent — the kind that may not.
const UNTIERED: &str = "66666666-6666-6666-6666-666666666666";
/// A `volume` parent — the *other* kind that may. Present because the rule is
/// `model_kind IN ('graduated','volume')` and a suite that only ever exercised
/// `graduated` would pass with the second literal dropped or misspelled on
/// either backend.
const TIERED_VOLUME: &str = "77777777-7777-7777-7777-777777777777";
/// A second market (EUR) of [`TIERED`]'s line version. Seeded only by the cases
/// that are about two markets of one line.
const TIERED_SIBLING: &str = "88888888-8888-8888-8888-888888888888";

/// Reject, **and** for the stated reason.
///
/// The fragment is not decoration. This table carries a dozen rejectors — five
/// `SQLite` triggers of its own, three CHECKs, a UNIQUE, a `band_id` PRIMARY
/// KEY, a foreign key, and the parent-side kind trigger its migration puts on
/// `pricing_charge_line_version` — and several of them can fire on the same
/// statement. An INSERT alone passes under the kind trigger, the append-only
/// trigger, the PK and the FK. A test that accepted any error would pass with
/// the guard it means to prove switched off; a band insert refused by the FK
/// says nothing at all about structural exclusivity, and one refused by the kind
/// trigger says nothing about the freeze.
async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the band guard must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("pricing_price_tier_band"),
        "the rejection must name the guard it came from, got: {message}"
    );
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// One price row under `charge_kind`'s line, in `currency`.
///
/// The seeder is idempotent on the line and its version, so a second call on
/// the same charge kind with another currency is a **sibling market of one line
/// version** — the shape the ladder's per-market identity has to be proven on.
async fn seed_price(
    conn: &DatabaseConnection,
    price_id: &str,
    charge_kind: &str,
    model_kind: &str,
    currency: &str,
    amount: &str,
) {
    let graph = common::seed_charge_graph_sql(
        conn,
        &common::SqlGraphSeed {
            charge_kind,
            currency,
            model_kind: Some(model_kind),
            lifecycle_state: "draft",
            created_by: ACTOR,
            ..common::SqlGraphSeed::new(TENANT, PLAN, PHASE, SKU)
        },
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, plan_revision, charge_line_id,
                line_version_id, market_price_id, amount_minor, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{price_id}', '{TENANT}', '{PLAN}', 0, '{}', '{}', '{}',
                {amount}, 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00')",
            graph.charge_line_id, graph.line_version_id, graph.market_price_id
        ),
    )
    .await;
}

/// One parent per model kind under test — `graduated`, `volume` and `flat` —
/// all draft, on distinct charge kinds so nothing depends on the scope-key
/// index.
///
/// The model kind is shared content of the line version and the charge kind is
/// an axis of the line, so each parent is a line of its own with its own
/// version — which is what "distinct charge kinds" already meant.
async fn seed_parents(conn: &DatabaseConnection) {
    for (price_id, charge_kind, model_kind, amount) in [
        (TIERED, "usage", "graduated", "NULL"),
        (TIERED_VOLUME, "one_time", "volume", "NULL"),
        (UNTIERED, "recurring", "flat", "1000"),
    ] {
        seed_price(conn, price_id, charge_kind, model_kind, "USD", amount).await;
    }
}

/// The line version behind one of [`seed_parents`]' price rows.
///
/// Derived the way the seeder derives it, so a case can address a parent's
/// version without threading ids through every signature.
fn version_of(price_id: &str) -> String {
    let charge_kind = match price_id {
        TIERED | TIERED_SIBLING => "usage",
        TIERED_VOLUME => "one_time",
        _ => "recurring",
    };
    common::sql_line_version_id(TENANT, PLAN, PHASE, SKU, charge_kind, 0)
}

/// One band of `price_id`'s ladder: its bounds beside its rate.
///
/// `from_qty` and `unit_price_nano` are **signed**, and deliberately. Typed
/// `u32`, no statement this file can build reaches
/// `chk_pricing_price_tier_band_from_qty` or
/// `chk_pricing_price_tier_band_unit_price` - both `>= 0` - so the module doc's
/// claim that each branch gets a case costs two of them their case by
/// construction of the helper rather than by omission.
fn insert_band(band_id: &str, price_id: &str, from_qty: i64, to_qty: &str, unit: i64) -> String {
    format!(
        "INSERT INTO pricing_price_tier_band (
            band_id, tenant_id, price_id, line_version_id, from_qty, to_qty, unit_price_nano)
         VALUES ('{band_id}', '{TENANT}', '{price_id}', '{}', {from_qty}, {to_qty}, {unit})",
        version_of(price_id)
    )
}

/// Move a band onto another price row, the only way the compound foreign key
/// admits: the price and the version it names move together.
fn re_point(band_id: &str, onto: &str) -> String {
    format!(
        "UPDATE pricing_price_tier_band SET price_id = '{onto}', line_version_id = '{}' \
         WHERE band_id = '{band_id}'",
        version_of(onto)
    )
}

/// The two `>= 0` CHECKs, each refused below its floor and accepted at it.
///
/// Zero is the boundary both admit: a band starting at 0 is the first band of
/// every ladder, and a `unit_price_nano` of 0 is a free tier - real authoring
/// shapes, so the floor has to be inclusive and a case has to say so.
#[tokio::test]
async fn the_two_floors_refuse_below_zero_and_admit_it() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    must_be_rejected(
        &conn,
        &insert_band(
            "aaaaaaa9-0000-0000-0000-000000000001",
            TIERED,
            -1,
            "100",
            50,
        ),
        "chk_pricing_price_tier_band_from_qty",
    )
    .await;
    must_be_rejected(
        &conn,
        &insert_band("aaaaaaa9-0000-0000-0000-000000000002", TIERED, 0, "100", -1),
        "chk_pricing_price_tier_band_unit_price",
    )
    .await;
    must_succeed(
        &conn,
        &insert_band("aaaaaaa9-0000-0000-0000-000000000003", TIERED, 0, "100", 0),
    )
    .await;
}

#[tokio::test]
async fn bands_are_permitted_only_on_a_tiered_parent() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    // Both halves of `model_kind IN ('graduated','volume')`, so neither literal
    // can be dropped without a failure here.
    must_succeed(
        &conn,
        &insert_band("aaaaaaa1-0000-0000-0000-000000000001", TIERED, 0, "100", 50),
    )
    .await;
    must_succeed(
        &conn,
        &insert_band(
            "aaaaaaa1-0000-0000-0000-000000000002",
            TIERED_VOLUME,
            0,
            "100",
            50,
        ),
    )
    .await;

    // A `flat` line's bands would be priced by nothing and read by nothing — a
    // silent mispricing, indistinguishable from a correct row until an invoice
    // is wrong. The kind flip that would otherwise be the way to reach this
    // state is already impossible on a published version, so the case is driven
    // through a second parent rather than an UPDATE of the first.
    must_be_rejected(
        &conn,
        &insert_band(
            "aaaaaaa1-0000-0000-0000-000000000003",
            UNTIERED,
            0,
            "100",
            50,
        ),
        "permitted only on a graduated or volume line version",
    )
    .await;

    let bands = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band",
    )
    .await;
    assert_eq!(bands, "2", "only the two tiered parents' ladders landed");
}

/// **A band is where it starts — within its own market.**
///
/// `uq_pricing_price_tier_band_lower_bound` is `(price_id, from_qty)`: one market
/// cannot hold two bands on one lower bound, and a sibling market of the *same
/// line version* starts its own ladder at 0 without asking anyone. The second
/// half is the whole point of the ladder being the market's, so it is asserted
/// rather than left to follow from the key's spelling.
#[tokio::test]
async fn a_lower_bound_identifies_a_band_within_its_own_market() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;
    seed_price(&conn, TIERED_SIBLING, "usage", "graduated", "EUR", "NULL").await;
    assert_eq!(
        version_of(TIERED),
        version_of(TIERED_SIBLING),
        "the sibling is a second market of the one line version"
    );

    must_succeed(
        &conn,
        &insert_band("aaaaaaa2-0000-0000-0000-000000000001", TIERED, 0, "100", 50),
    )
    .await;
    must_be_rejected(
        &conn,
        &insert_band("aaaaaaa2-0000-0000-0000-000000000002", TIERED, 0, "300", 40),
        "UNIQUE constraint failed",
    )
    .await;

    // The sibling's ladder shares nothing with it: not the break-points, not the
    // number of bands, not the rates.
    must_succeed(
        &conn,
        &insert_band(
            "aaaaaaa2-0000-0000-0000-000000000003",
            TIERED_SIBLING,
            0,
            "50",
            45,
        ),
    )
    .await;
    must_succeed(
        &conn,
        &insert_band(
            "aaaaaaa2-0000-0000-0000-000000000004",
            TIERED_SIBLING,
            50,
            "500",
            35,
        ),
    )
    .await;
    must_succeed(
        &conn,
        &insert_band(
            "aaaaaaa2-0000-0000-0000-000000000005",
            TIERED_SIBLING,
            500,
            "NULL",
            25,
        ),
    )
    .await;

    for (price_id, expected) in [(TIERED, "1"), (TIERED_SIBLING, "3")] {
        let bands = scalar(
            &conn,
            &format!(
                "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band \
                 WHERE price_id = '{price_id}'"
            ),
        )
        .await;
        assert_eq!(bands, expected, "{price_id} carries its own ladder");
    }
}

#[tokio::test]
async fn a_band_covers_a_quantity_or_is_open_at_the_top() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    // A band that covers nothing is always an authoring mistake and it makes
    // the set's contiguity ambiguous.
    must_be_rejected(
        &conn,
        &insert_band(
            "aaaaaaa3-0000-0000-0000-000000000001",
            TIERED,
            100,
            "100",
            50,
        ),
        "chk_pricing_price_tier_band_width",
    )
    .await;
    must_be_rejected(
        &conn,
        &insert_band(
            "aaaaaaa3-0000-0000-0000-000000000002",
            TIERED,
            100,
            "40",
            50,
        ),
        "chk_pricing_price_tier_band_width",
    )
    .await;

    // NULL is the open top — a state of the band, not an absent value.
    must_succeed(
        &conn,
        &insert_band(
            "aaaaaaa3-0000-0000-0000-000000000003",
            TIERED,
            100,
            "NULL",
            50,
        ),
    )
    .await;

    let top = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band WHERE to_qty IS NULL",
    )
    .await;
    assert_eq!(top, "1", "the open-topped band is the one that landed");
}

#[tokio::test]
async fn a_band_freezes_when_its_parent_does() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    let mutable = "aaaaaaa4-0000-0000-0000-000000000001";
    let doomed = "aaaaaaa4-0000-0000-0000-000000000002";
    must_succeed(&conn, &insert_band(mutable, TIERED, 0, "100", 50)).await;
    must_succeed(&conn, &insert_band(doomed, TIERED, 100, "NULL", 40)).await;

    // While the parent is draft the band set is authoring material — bounds and
    // rate alike.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price_tier_band SET unit_price_nano = 45, to_qty = 90 \
             WHERE band_id = '{mutable}'"
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_price_tier_band WHERE band_id = '{doomed}'"),
    )
    .await;

    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'published' WHERE price_id = '{TIERED}'"
        ),
    )
    .await;

    // Bands carry no `lifecycle_state` of their own; the parent's is the
    // referent. Without this the band set of a frozen row could be rewritten
    // under an unchanged `pricing_price`, and the projector's warm re-drive —
    // which reads truth rows — would re-materialize the same `CatalogVersion`
    // at different money. A moved **bound** is the same fault as a moved rate:
    // both are what the market charges.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price_tier_band SET unit_price_nano = 1 WHERE band_id = '{mutable}'"
        ),
        "UPDATE of a band under a non-draft price row is not permitted",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price_tier_band SET to_qty = 80 WHERE band_id = '{mutable}'"),
        "UPDATE of a band under a non-draft price row is not permitted",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_price_tier_band WHERE band_id = '{mutable}'"),
        "DELETE of a band under a non-draft price row is not permitted",
    )
    .await;

    // And the verb that ADDS money to a frozen row. The kind trigger fires on
    // INSERT but reads only `model_kind`, which is still `graduated` here — so
    // without the INSERT arm this statement would land a new priced band under
    // a published row. The fragment is what distinguishes the arm under test
    // from the kind trigger and the foreign key, either of which could also
    // refuse an INSERT into this table.
    must_be_rejected(
        &conn,
        &insert_band(
            "aaaaaaa4-0000-0000-0000-000000000003",
            TIERED,
            100,
            "NULL",
            30,
        ),
        "INSERT of a band under a non-draft price row is not permitted",
    )
    .await;

    let price = scalar(
        &conn,
        &format!(
            "SELECT CAST(unit_price_nano AS TEXT) || '/' || CAST(to_qty AS TEXT) AS v \
             FROM pricing_price_tier_band WHERE band_id = '{mutable}'"
        ),
    )
    .await;
    assert_eq!(price, "45/90", "no rejected statement may have landed");
    let bands = scalar(
        &conn,
        &format!(
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band \
             WHERE price_id = '{TIERED}'"
        ),
    )
    .await;
    assert_eq!(bands, "1", "the frozen row's band set did not grow");
}

#[tokio::test]
async fn a_re_pointed_band_is_judged_against_its_new_parents_kind() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    let band = "aaaaaaa5-0000-0000-0000-000000000001";
    must_succeed(&conn, &insert_band(band, TIERED, 0, "100", 50)).await;

    // The kind rule cares only about where a band ends up, so it reads the
    // `NEW` row. Moving this band onto the `flat` parent is the only way to
    // observe that choice: nothing else in the suite distinguishes it from the
    // `OLD` one, and both parents are draft, so the append-only arms stay out
    // of the way.
    must_be_rejected(
        &conn,
        &re_point(band, UNTIERED),
        "permitted only on a graduated or volume line version",
    )
    .await;

    // A move onto the other tiered kind is a legitimate authoring edit.
    must_succeed(&conn, &re_point(band, TIERED_VOLUME)).await;

    let parent = scalar(
        &conn,
        &format!(
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band \
             WHERE price_id = '{TIERED_VOLUME}'"
        ),
    )
    .await;
    assert_eq!(parent, "1", "only the legal move landed");
}

#[tokio::test]
async fn a_version_that_still_prices_bands_may_not_leave_the_tiered_kinds() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;
    seed_price(&conn, TIERED_SIBLING, "usage", "graduated", "EUR", "NULL").await;

    let version = version_of(TIERED);
    let own = "aaaaaaa6-0000-0000-0000-000000000001";
    let siblings = "aaaaaaa6-0000-0000-0000-000000000002";
    must_succeed(&conn, &insert_band(own, TIERED, 0, "100", 50)).await;
    must_succeed(&conn, &insert_band(siblings, TIERED_SIBLING, 0, "NULL", 45)).await;

    // The other end of structural exclusivity, and the one the child-side arms
    // cannot see: they judge a band as it arrives, and these bands arrived
    // legally. Flipping the version reaches the forbidden pair — a `flat` line
    // with priced bands nothing reads and nothing applies — from the side
    // nothing was watching, and leaves it there.
    //
    // The statement is raw SQL against a **draft** version on purpose: a draft
    // is where `model_kind` is still movable at all, so this is the only
    // reachable shape of the defect, and the repositories are not the only thing
    // that can issue an UPDATE.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_charge_line_version SET model_kind = 'flat' \
             WHERE line_version_id = '{version}'"
        ),
        "may not leave the graduated or volume kinds",
    )
    .await;
    // Kindless is the same fault: `model_kind` is nullable because a draft may
    // be authored before its kind is, and a NULL kind reads bands nowhere.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_charge_line_version SET model_kind = NULL \
             WHERE line_version_id = '{version}'"
        ),
        "may not leave the graduated or volume kinds",
    )
    .await;

    // The moves that must still work, without which the guard would be a ban.
    // Between the two tiered kinds the bands stay meaningful.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_charge_line_version SET model_kind = 'volume' \
             WHERE line_version_id = '{version}'"
        ),
    )
    .await;
    // A non-kind edit of a banded price row is untouched by the rule.
    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET row_version = 1 WHERE price_id = '{TIERED}'"),
    )
    .await;

    // **Every market's ladder has to be gone, not one.** The version is shared,
    // so clearing the market an edit arrived through leaves the sibling's bands
    // under a kind that reads none — which is why the line's own door clears
    // them all before it moves the kind.
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_price_tier_band WHERE band_id = '{own}'"),
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_charge_line_version SET model_kind = 'flat' \
             WHERE line_version_id = '{version}'"
        ),
        "may not leave the graduated or volume kinds",
    )
    .await;
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_price_tier_band WHERE band_id = '{siblings}'"),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_charge_line_version SET model_kind = 'flat' \
             WHERE line_version_id = '{version}'"
        ),
    )
    .await;

    let kind = scalar(
        &conn,
        &format!(
            "SELECT model_kind AS v FROM pricing_charge_line_version \
             WHERE line_version_id = '{version}'"
        ),
    )
    .await;
    assert_eq!(kind, "flat", "only the legal moves landed");
}

#[tokio::test]
async fn a_band_may_not_be_re_pointed_onto_a_frozen_parent() {
    let conn = migrated_db().await;
    seed_parents(&conn).await;

    let band = "aaaaaaa7-0000-0000-0000-000000000001";
    must_succeed(&conn, &insert_band(band, TIERED, 0, "100", 50)).await;
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'published' \
             WHERE price_id = '{TIERED_VOLUME}'"
        ),
    )
    .await;

    // Re-pointing is how you would append a band to a frozen set without ever
    // issuing an INSERT: the band's own parent is draft, so the OLD-side arm
    // permits the edit, and the target's line is `volume`, so the kind rule
    // permits it too. Only reading the prospective parent's `lifecycle_state`
    // refuses it.
    must_be_rejected(
        &conn,
        &re_point(band, TIERED_VOLUME),
        "UPDATE of a band under a non-draft price row is not permitted",
    )
    .await;

    let parent = scalar(
        &conn,
        &format!(
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price_tier_band \
             WHERE price_id = '{TIERED}'"
        ),
    )
    .await;
    assert_eq!(parent, "1", "the band stayed on its draft parent");
}
