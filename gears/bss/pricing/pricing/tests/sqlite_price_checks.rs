//! The row `CHECK` constraints on `pricing_price`, proven against a real
//! database.
//!
//! A `CHECK` that is silently wrong is a `CHECK` that never refuses anything,
//! and nothing else in the suite notices: deleting
//! `chk_pricing_price_package_fields_kind` from both backends left every other
//! test in this crate green, because every row the repository writes satisfies
//! it anyway. These constraints exist for the rows the repository does **not**
//! write — a migration script, a console session, a future slice's writer — so
//! only a test that reaches past the repository can say they are there.
//!
//! **All twenty** of them get cases, and the paragraph that used to stand here
//! explaining why five of them did not was wrong in the way the paragraph above
//! describes. It read: `model_kind`, `charge_kind`, `price_eligibility`,
//! `lifecycle_state` and the cohort / eligibility biconditional "already fail a
//! repository test the moment their CHECK is wrong, because the repository
//! writes across their whole range". A repository that writes across a column's
//! whole range catches a CHECK that is too **narrow** — it would start refusing
//! a value the gear authors — and can never catch one that has stopped
//! refusing, because it never offers a value outside the range. Rewriting all
//! twelve of the unproven constraints to `CHECK (1 = 1)` left this crate green
//! at 322 tests; so did neutering the two grandfathering ones. The claim was
//! exactly backwards, and it exempted the constraints
//! `m20260802_000002_create_pricing_price` leans on hardest: the repository's
//! `CorruptRow` reading of a foreign token is justified "only if the column
//! cannot hold such a token in the first place", which is a statement about the
//! schema that only a test reaching past the repository can make.
//!
//! Four rules get cases here, and the token sets below are written out as
//! literals rather than derived from the domain enums on purpose — a pin that
//! moved with the thing it pins would agree with any schema at all.
//!
//! **The package half of structural exclusivity** (`design/03-price-structure.md`
//! §6): package block fields are permitted on `model_kind = 'package'` and
//! nowhere else. The **kindless** row is the case worth naming, and it is the
//! one the constraint used to admit: `model_kind` is nullable, so the shorter
//! `OR model_kind = 'package'` evaluates to NULL on such a row, and a NULL
//! CHECK result counts as satisfied on both engines. The band half of the same
//! §6 rule always refused that row explicitly — its message spells the state
//! `kindless` — so for as long as the halves disagreed, one shape of unpriceable
//! row was reachable through the half that reads a row and not through the half
//! that reads a parent.
//!
//! **Every token column**, whose refusals the repository's `CorruptRow` reading
//! depends on: it reads each column back through the inverse of a domain enum's
//! `as_str()` and calls anything else an invariant breach rather than a caller
//! mistake, which is only true while the column cannot hold such a value. The
//! `NOT NULL` ones are driven by INSERT (a row per token, each on a region of
//! its own) and the nullable ones by UPDATE of one freely-mutable draft.
//!
//! **Every quantity and money column**, by its boundary pair rather than by one
//! value: `>= 0`, `> 0` and `>= 1` are three different constraints that agree on
//! every input except one, so a case that only refused `-1` would pass against
//! any of the three.
//!
//! **The grandfathering pair**: the cohort / eligibility biconditional in both
//! directions, and the horizon that only the grandfathered class may carry.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed, scalar};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const PHASE: &str = "33333333-3333-3333-3333-333333333333";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const SEED: &str = "55555555-5555-5555-5555-555555555555";

/// A token no column's enumeration contains, so one literal drives every
/// negative case below.
const FOREIGN_TOKEN: &str = "sum_of_squares";

/// Refused, **and** by the named constraint.
///
/// Naming it is what makes the case a proof. `pricing_price` carries twenty
/// `CHECK` constraints and two partial `UNIQUE` indexes, several of which can
/// answer the same statement — an INSERT that lands on an occupied scope key is
/// refused whatever its `model_kind` says — so a test that accepted any error
/// would pass against a table whose constraint under test had been deleted.
/// `SQLite` reports a named `CHECK` as `CHECK constraint failed: <name>`.
async fn must_violate(conn: &DatabaseConnection, sql: &str, constraint: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the constraint must refuse: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("CHECK constraint failed"),
        "the refusal must come from a CHECK, got: {message}"
    );
    assert!(
        message.contains(constraint),
        "the refusal must be `{constraint}`, got: {message}"
    );
}

/// One draft usage row, on the base scope key, carrying none of the columns
/// under test.
///
/// A draft is freely mutable, so the token cases drive this single row through
/// every value rather than inserting one row per token — which would need a
/// distinct scope key per token and would prove something about the key index
/// instead.
async fn seed(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, model_kind, meter, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{SEED}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'usage', 'per_unit', 'api_calls', 'draft', '{ACTOR}',
                '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
}

/// An INSERT of a row that differs from the seed only in the columns given —
/// `region` keeps it on a scope key of its own, so nothing here can be refused
/// by a unique index.
fn insert_row(price_id: &str, region: &str, columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO pricing_price (
            price_id, tenant_id, plan_id, currency, region, phase,
            charge_kind, lifecycle_state, created_by, created_at_utc{columns})
         VALUES ('{price_id}', '{TENANT}', '{PLAN}', 'USD', '{region}', '{PHASE}',
            'usage', 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00'{values})"
    )
}

/// An INSERT whose four `NOT NULL` token columns are all caller-chosen, `column`
/// carrying `token` and the rest their defaults.
///
/// These four cannot be driven the way the nullable ones are. Three of them are
/// scope-key columns, so an UPDATE moves the row's key rather than its content;
/// `lifecycle_state` is guarded by the append-only triggers the moment it leaves
/// `draft`, which would answer before the CHECK ever did. So each token gets a
/// row, on a `region` — and therefore a scope key — of its own.
///
/// `cohort` follows `price_eligibility` because the biconditional binds them: a
/// helper that wrote `none` under `existing_grandfathered` would have every
/// positive case for that token refused by a constraint that is not the one
/// under test.
fn insert_token_row(seq: usize, column: &str, token: &str) -> String {
    let chosen = |name: &str, default: &str| -> String {
        if name == column { token } else { default }.to_owned()
    };
    let eligibility = chosen("price_eligibility", "all_subscriptions");
    let cohort = if eligibility == "existing_grandfathered" {
        "1780000000000"
    } else {
        "none"
    };
    format!(
        "INSERT INTO pricing_price (
            price_id, tenant_id, plan_id, currency, region, phase,
            price_overlay, price_eligibility, cohort, charge_kind,
            lifecycle_state, created_by, created_at_utc)
         VALUES ('cccc0000-0000-0000-0000-{seq:012}', '{TENANT}', '{PLAN}', 'USD',
            'R{seq}', '{PHASE}', '{}', '{eligibility}', '{cohort}', '{}', '{}',
            '{ACTOR}', '2026-08-02 10:00:00 +00:00')",
        chosen("price_overlay", "base"),
        chosen("charge_kind", "usage"),
        chosen("lifecycle_state", "draft"),
    )
}

#[tokio::test]
async fn package_block_fields_need_the_kind_that_gives_them_meaning() {
    let conn = migrated_db().await;

    // The kindless row. `model_kind` is nullable — a draft may be authored
    // before its kind is — so this row's package fields would once have landed:
    // `FALSE OR NULL` is NULL, and a NULL CHECK result is satisfied. Nothing
    // downstream reads a block on a row with no kind, and nothing prices it, so
    // it is a silent unpriceable row rather than a loud one.
    must_violate(
        &conn,
        &insert_row(
            "aaaa0001-0000-0000-0000-000000000001",
            "EU",
            ", package_size, package_price_minor",
            ", 100, 5000",
        ),
        "chk_pricing_price_package_fields_kind",
    )
    .await;

    // And the case the constraint always did refuse: a block on a kind whose
    // money lives somewhere else. One field is enough — the rule is about the
    // pair being present at all, not about the pair being complete.
    must_violate(
        &conn,
        &insert_row(
            "aaaa0001-0000-0000-0000-000000000002",
            "US",
            ", model_kind, amount_minor, package_price_minor",
            ", 'flat', 1000, 5000",
        ),
        "chk_pricing_price_package_fields_kind",
    )
    .await;
    must_violate(
        &conn,
        &insert_row(
            "aaaa0001-0000-0000-0000-000000000003",
            "APAC",
            ", model_kind, package_size",
            ", 'graduated', 100",
        ),
        "chk_pricing_price_package_fields_kind",
    )
    .await;

    // The positive control, without which all of the above would pass against a
    // constraint that refused every row: on `package` the block is the price.
    must_succeed(
        &conn,
        &insert_row(
            "aaaa0001-0000-0000-0000-000000000004",
            "LATAM",
            ", model_kind, package_size, package_price_minor",
            ", 'package', 100, 5000",
        ),
    )
    .await;
    // As is a kindless row that carries no block at all: the rule forbids the
    // pairing, not the absent kind.
    must_succeed(
        &conn,
        &insert_row("aaaa0001-0000-0000-0000-000000000005", "MEA", "", ""),
    )
    .await;

    let landed = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(landed, "2", "only the two permitted rows landed");
}

#[tokio::test]
async fn each_not_null_token_column_holds_only_the_tokens_its_enum_renders() {
    /// Column, the constraint that guards it, and every token the domain enum
    /// behind it renders.
    ///
    /// `lifecycle_state` is the one whose list is **narrower than its enum**.
    /// `domain::lifecycle::LifecycleState` is shared with plan revisions, which
    /// reach `retired`; the price-row state machine
    /// (`design/03-price-structure.md` §4) has three states and no `retired`
    /// edge, and a `retired` price row would fall outside both partial `UNIQUE`
    /// indexes — so the key would read as free and take a second published row
    /// beside it.
    const COLUMNS: [(&str, &str, &[&str]); 4] = [
        (
            "lifecycle_state",
            "chk_pricing_price_lifecycle_state",
            &["draft", "published", "superseded"],
        ),
        ("price_overlay", "chk_pricing_price_overlay", &["base"]),
        (
            "price_eligibility",
            "chk_pricing_price_eligibility",
            &[
                "all_subscriptions",
                "new_subscriptions_only",
                "existing_grandfathered",
            ],
        ),
        (
            "charge_kind",
            "chk_pricing_price_charge_kind",
            &["recurring", "usage", "one_time", "one_time_setup"],
        ),
    ];

    let conn = migrated_db().await;
    let mut seq = 0;
    let mut landed = 0;
    for (column, constraint, tokens) in COLUMNS {
        // Every token the enum renders, one row each. A constraint listing
        // three of four, or listing another column's set, looks exactly like a
        // working one until the missing token is authored — and
        // `new_subscriptions_only` is precisely that case: a whole normative
        // eligibility class the column refused.
        for token in tokens {
            seq += 1;
            landed += 1;
            must_succeed(&conn, &insert_token_row(seq, column, token)).await;
        }
        // And the token no enum renders, which is the refusal the repository's
        // `CorruptRow` reading rests on. It is a claim about the schema, not
        // about the repository, so only a statement the repository never issues
        // can make it.
        seq += 1;
        must_violate(
            &conn,
            &insert_token_row(seq, column, FOREIGN_TOKEN),
            constraint,
        )
        .await;
    }

    // `retired` is the one token this table refuses that is nonetheless a real
    // value of the enum behind the column, so a foreign-token case cannot reach
    // it and a list derived from the enum would admit it. It is a **plan
    // revision's** terminal state; the price-row machine has no `retired` edge,
    // and a `retired` price row would sit outside both partial `UNIQUE` indexes
    // with its key reading free.
    seq += 1;
    must_violate(
        &conn,
        &insert_token_row(seq, "lifecycle_state", "retired"),
        "chk_pricing_price_lifecycle_state",
    )
    .await;

    // A CHECK that refused a statement which took effect anyway would be
    // indistinguishable above from one that worked.
    let stored = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(stored, landed.to_string(), "exactly the legal rows landed");
}

#[tokio::test]
async fn each_nullable_token_column_holds_only_the_tokens_its_enum_renders() {
    /// Column, the constraint that guards it, and every token the domain enum
    /// behind it renders.
    const COLUMNS: [(&str, &str, &[&str]); 8] = [
        (
            "model_kind",
            "chk_pricing_price_model_kind",
            &["flat", "per_unit", "graduated", "volume", "package"],
        ),
        (
            "billing_timing",
            "chk_pricing_price_billing_timing",
            &["advance", "arrears"],
        ),
        (
            "quantity_source",
            "chk_pricing_price_quantity_source",
            &["subscription_seat_count", "manual"],
        ),
        (
            "billing_granularity",
            "chk_pricing_price_billing_granularity",
            &[
                "per_second",
                "per_minute",
                "per_hour",
                "per_day",
                "whole_unit",
            ],
        ),
        (
            "aggregation_function",
            "chk_pricing_price_aggregation_function",
            &["sum", "peak", "time_weighted"],
        ),
        (
            "aggregation_granularity",
            "chk_pricing_price_aggregation_granularity",
            &["hour", "day"],
        ),
        (
            "tier_aggregation_window",
            "chk_pricing_price_tier_aggregation_window",
            &[
                "calendar_month",
                "invoice_period",
                "subscription_lifetime",
                "per_event",
            ],
        ),
        (
            "tier_qualification_window",
            "chk_pricing_price_tier_qualification_window",
            &["current", "trailing_period"],
        ),
    ];

    let conn = migrated_db().await;
    seed(&conn).await;

    // A draft is freely mutable, so one row is driven through every value
    // rather than a row per token — which would need a distinct scope key per
    // token and would prove something about the key index instead.
    //
    // Each column gets all four moves, and each of the four is load-bearing.
    // *Every* token the enum renders, one at a time: a constraint listing four
    // of five, or listing another column's set, looks exactly like a working
    // one until the missing token is authored. NULL, because absent is a state
    // of all eight — the row is authored before it is publishable, and each of
    // them is required only for some kinds. A token no enum renders, which is
    // the refusal the repository's `CorruptRow` reading rests on: it calls a
    // foreign token an invariant breach rather than a caller mistake, and that
    // is a claim about the schema, not about the repository. And a read-back,
    // because a CHECK that refuses a statement which took effect anyway would
    // be indistinguishable here from one that worked.
    for (column, constraint, tokens) in COLUMNS {
        for token in tokens {
            must_succeed(
                &conn,
                &format!("UPDATE pricing_price SET {column} = '{token}' WHERE price_id = '{SEED}'"),
            )
            .await;
        }
        must_succeed(
            &conn,
            &format!("UPDATE pricing_price SET {column} = NULL WHERE price_id = '{SEED}'"),
        )
        .await;
        must_violate(
            &conn,
            &format!(
                "UPDATE pricing_price SET {column} = '{FOREIGN_TOKEN}' WHERE price_id = '{SEED}'"
            ),
            constraint,
        )
        .await;
        let stored = scalar(
            &conn,
            &format!(
                "SELECT coalesce({column}, 'null') AS v FROM pricing_price \
                 WHERE price_id = '{SEED}'"
            ),
        )
        .await;
        assert_eq!(
            stored, "null",
            "the refused UPDATE of {column} may not land"
        );
    }
}

#[tokio::test]
async fn each_quantity_and_money_column_refuses_the_value_just_past_its_bound() {
    /// Column, the constraint that guards it, the smallest value it admits, and
    /// the largest it refuses.
    ///
    /// The **pair** is the case. `>= 0`, `> 0` and `>= 1` agree on every input
    /// but one, so a case that only offered `-1` would pass against all three
    /// and a bound copied from the wrong neighbour would look correct. Naming
    /// both sides of the step pins which of the three each column carries.
    const BOUNDS: [(&str, &str, &str, &str); 5] = [
        (
            "amount_minor",
            "chk_pricing_price_amount_non_negative",
            "0",
            "-1",
        ),
        (
            "manual_quantity",
            "chk_pricing_price_manual_quantity",
            "0",
            "-1",
        ),
        (
            "max_hold_granules",
            "chk_pricing_price_max_hold_granules",
            "1",
            "0",
        ),
        ("package_size", "chk_pricing_price_package_size", "1", "0"),
        (
            "package_price_minor",
            "chk_pricing_price_package_price",
            "0",
            "-1",
        ),
    ];

    let conn = migrated_db().await;
    seed(&conn).await;
    // The block columns are reachable at all only on a `package` row: on any
    // other kind `chk_pricing_price_package_fields_kind` answers first, and the
    // case would prove that constraint twice instead of these two once.
    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET model_kind = 'package' WHERE price_id = '{SEED}'"),
    )
    .await;

    for (column, constraint, admitted, refused) in BOUNDS {
        must_succeed(
            &conn,
            &format!("UPDATE pricing_price SET {column} = {admitted} WHERE price_id = '{SEED}'"),
        )
        .await;
        must_violate(
            &conn,
            &format!("UPDATE pricing_price SET {column} = {refused} WHERE price_id = '{SEED}'"),
            constraint,
        )
        .await;
        // The refused value may not have landed, and absent is a state of every
        // one of these columns: each is required only for some kinds.
        let stored = scalar(
            &conn,
            &format!(
                "SELECT CAST({column} AS TEXT) AS v FROM pricing_price WHERE price_id = '{SEED}'"
            ),
        )
        .await;
        assert_eq!(
            stored, admitted,
            "the refused UPDATE of {column} may not land"
        );
        must_succeed(
            &conn,
            &format!("UPDATE pricing_price SET {column} = NULL WHERE price_id = '{SEED}'"),
        )
        .await;
    }
}

#[tokio::test]
async fn the_cohort_pairing_holds_both_ways_and_the_horizon_needs_its_class() {
    let conn = migrated_db().await;

    // Forward direction of the biconditional: a cohort on a row of a class that
    // retains nobody. Such a row sits on a key no resolution class ever selects
    // — published, and never priced from.
    for (seq, eligibility) in ["all_subscriptions", "new_subscriptions_only"]
        .into_iter()
        .enumerate()
    {
        must_violate(
            &conn,
            &insert_row(
                &format!("dddd0000-0000-0000-0000-00000000000{seq}"),
                &format!("F{seq}"),
                ", price_eligibility, cohort",
                &format!(", '{eligibility}', '1780000000000'"),
            ),
            "chk_pricing_price_cohort_eligibility",
        )
        .await;
    }

    // Reverse direction, and the more damaging one: a grandfathered row with no
    // generation lands on the `all_subscriptions` successor's own key and, being
    // immutable, occupies the key the next reprice needs.
    must_violate(
        &conn,
        &insert_row(
            "dddd0000-0000-0000-0000-000000000010",
            "F10",
            ", price_eligibility, cohort",
            ", 'existing_grandfathered', 'none'",
        ),
        "chk_pricing_price_cohort_eligibility",
    )
    .await;

    // The horizon is the grandfathered class's alone: it expires a *retained
    // generation*, and the other two classes retain nobody. Neither statement
    // trips the biconditional — both carry `cohort = 'none'` under a
    // non-grandfathered class — so the constraint under test is the only one
    // that can be answering.
    for (seq, eligibility) in ["all_subscriptions", "new_subscriptions_only"]
        .into_iter()
        .enumerate()
    {
        must_violate(
            &conn,
            &insert_row(
                &format!("dddd0000-0000-0000-0000-00000000002{seq}"),
                &format!("G{seq}"),
                ", price_eligibility, grandfather_until",
                &format!(", '{eligibility}', '2027-01-01 00:00:00 +00:00'"),
            ),
            "chk_pricing_price_grandfather_until",
        )
        .await;
    }

    // The positive controls, without which every case above would pass against
    // a pair of constraints that refused every row: the grandfathered class
    // carries both a generation and a horizon, and the default class carries
    // neither.
    must_succeed(
        &conn,
        &insert_row(
            "dddd0000-0000-0000-0000-000000000030",
            "H0",
            ", price_eligibility, cohort, grandfather_until",
            ", 'existing_grandfathered', '1780000000000', '2027-01-01 00:00:00 +00:00'",
        ),
    )
    .await;
    must_succeed(
        &conn,
        &insert_row(
            "dddd0000-0000-0000-0000-000000000031",
            "H1",
            ", price_eligibility",
            ", 'new_subscriptions_only'",
        ),
    )
    .await;

    let landed = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(landed, "2", "only the two permitted rows landed");
}
