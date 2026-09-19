//! The row `CHECK` constraints on `pricing_price` — and the one partial
//! `UNIQUE` index that is a slice rule rather than a key — proven against a
//! real database.
//!
//! A `CHECK` that is silently wrong is a `CHECK` that never refuses anything,
//! and nothing else in the suite notices: deleting
//! `chk_pricing_price_package_fields_kind` from both backends left every other
//! test in this crate green, because every row the repository writes satisfies
//! it anyway. These constraints exist for the rows the repository does **not**
//! write — a migration script, a console session, a future slice's writer — so
//! only a test that reaches past the repository can say they are there.
//!
//! **Every one** of them gets a case, and no constraint is exempted on the
//! ground that the repository already covers it. That exemption is exactly
//! backwards: a repository which writes across a column's whole range catches a
//! CHECK that is too **narrow** — it would start refusing a value the gear
//! authors — and can never catch one that has stopped refusing, because it
//! never offers a value outside the range. Rewriting the unproven constraints
//! to `CHECK (1 = 1)` leaves this crate green, and so does neutering the two
//! grandfathering ones. The exemption would fall hardest on the constraints
//! `pricing_price` leans on most: the repository's
//! `CorruptRow` reading of a foreign token is justified "only if the column
//! cannot hold such a token in the first place", which is a statement about the
//! schema that only a test reaching past the repository can make.
//!
//! The classes that get cases here are the package pairing, the token columns, the
//! quantity and money bounds, the grandfathering pair, meter injectivity and the
//! separator pair. Their token sets are written out as literals rather than derived
//! from the domain enums on purpose — a pin that moved with the thing it pins would
//! agree with any schema at all.
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
//!
//! **Meter injectivity** (`design/02-plan-definition.md` §6,
//! `inst-cmp-injective` / D-103), the one rule here that a `UNIQUE` index says
//! rather than a `CHECK`: one priced line per `(meter, dimension_key)` per
//! scope-key slice. Two of its arms are only reachable from a suite that writes
//! its own rows. The **undimensioned** pair is one — `dimension_key` is
//! `NOT NULL DEFAULT ''` so that two of them collide, and a case built only
//! from dimensioned rows would pass just as happily against the nullable column
//! whose distinct NULLs let the commonest duplicate line through. The other is
//! the pair that disagrees about `charge_kind`, which no writer in this gear
//! would produce and which is the only shape that tells this index apart from
//! the scope-key one beside it.

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
const SEED: &str = "55555555-5555-5555-5555-555555555555";

/// A token no column's enumeration contains, so one literal drives every
/// negative case below.
const FOREIGN_TOKEN: &str = "sum_of_squares";

/// Refused, **and** by the named constraint.
///
/// Naming it is what makes the case a proof. `pricing_price` carries a `CHECK`
/// per column rule and three partial `UNIQUE` indexes, several of which can
/// answer the same statement — an INSERT that lands on an occupied scope key is
/// refused whatever its `model_kind` says — so a test that accepted any error
/// would pass against a table whose constraint under test had been deleted.
/// `SQLite` reports a named `CHECK` as `CHECK constraint failed: <name>`.
/// Refused by a **trigger**, naming its sentence.
///
/// Two of the rules this file proves span tables now — a block price needs the
/// `package` kind, a grandfather horizon needs the `existing_grandfathered`
/// class — and the column each rule reads lives on `pricing_charge_line_version`
/// or `pricing_charge_line` while the column it guards stayed on
/// `pricing_price`. A `CHECK` cannot see another table, so those two moved to
/// triggers, and a case demanding `CHECK constraint failed` would report them
/// missing when they are merely expressed differently.
async fn must_abort(conn: &DatabaseConnection, sql: &str, sentence: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the guard must refuse: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains(sentence),
        "the refusal must be `{sentence}`, got: {message}"
    );
}

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

/// The one charge line the version cases below hang their versions off.
const CHARGE_LINE: &str = "0000cc00-0000-0000-0000-000000000001";

/// The line and the two markets the package cases price against.
///
/// One line, because the rules under test are the *version's* and the *row's*,
/// not the line's — a line per case would vary an axis nothing here is about.
/// Two markets, because a draft row is unique per market and the case needs two.
async fn ensure_line(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT OR IGNORE INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, \
             phase, charge_kind, sku_id) VALUES \
             ('{TENANT}', '{CHARGE_LINE}', '{PLAN}', '{PHASE}', 'usage', '{SKU}')"
        ),
    )
    .await;
    for (region, market) in [
        ("pk", "0000dd00-0000-0000-0000-000000000001"),
        ("nk", "0000dd00-0000-0000-0000-000000000002"),
    ] {
        must_succeed(
            conn,
            &format!(
                "INSERT OR IGNORE INTO pricing_market_price (tenant_id, market_price_id, \
                 charge_line_id, currency, region) VALUES \
                 ('{TENANT}', '{market}', '{CHARGE_LINE}', 'USD', '{region}')"
            ),
        )
        .await;
    }
}

/// A version of [`LINE`], at a revision derived from its id.
///
/// One line holds one version per plan revision, so the id's last digit doubles
/// as the revision — which keeps the cases independent without making the
/// revision a thing any of them is about.
fn insert_version(version_id: &str, model_kind: &str, columns: &str, values: &str) -> String {
    let revision = version_id.chars().last().unwrap_or('1');
    format!(
        "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, charge_line_id, \
         plan_revision, lifecycle_state, model_kind, created_by, created_at_utc, \
         row_version{columns}) \
         VALUES ('{TENANT}', '{version_id}', '{CHARGE_LINE}', {revision}, 'draft', {model_kind}, \
         '{ACTOR}', '2026-08-02 10:00:00 +00:00', 0{values})"
    )
}

/// A draft price row against one version, in one of [`ensure_line`]'s markets.
fn insert_price_on(
    price_id: &str,
    version_id: &str,
    region: &str,
    columns: &str,
    values: &str,
) -> String {
    let market = if region == "pk" {
        "0000dd00-0000-0000-0000-000000000001"
    } else {
        "0000dd00-0000-0000-0000-000000000002"
    };
    format!(
        "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, \
         charge_line_id, line_version_id, market_price_id, lifecycle_state, created_by, \
         created_at_utc{columns}) \
         VALUES ('{price_id}', '{TENANT}', '{PLAN}', 0, '{CHARGE_LINE}', '{version_id}', '{market}', \
         'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00'{values})"
    )
}

/// The seeded version and market the drivable cases mutate.
const SEED_VERSION: &str = "0000cc00-0000-0000-0000-00000000000e";
const SEED_MARKET: &str = "0000dd00-0000-0000-0000-00000000000e";

/// One draft usage line — its version and one draft price row — carrying none
/// of the columns under test.
///
/// A draft is freely mutable, so the token cases drive this single line through
/// every value rather than inserting one row per token — which would need a
/// distinct scope key per token and would prove something about the key index
/// instead.
///
/// **Three rows rather than one**, because the columns those cases drive have
/// three owners now: the eight logical axes are `pricing_charge_line`'s, the
/// shared calculation is its version's, and the money is the price row's.
async fn seed(conn: &DatabaseConnection) {
    ensure_line(conn).await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES ('{TENANT}', '{SEED_MARKET}', '{CHARGE_LINE}', 'USD', 'EU')"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, meter, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}', '{SEED_VERSION}', '{CHARGE_LINE}', 0, 'draft', 'per_unit', \
              'api_calls', '{ACTOR}', '2026-08-02 10:00:00 +00:00', 0)"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, \
             charge_line_id, line_version_id, market_price_id, lifecycle_state, created_by, \
             created_at_utc) VALUES \
             ('{SEED}', '{TENANT}', '{PLAN}', 0, '{CHARGE_LINE}', '{SEED_VERSION}', \
              '{SEED_MARKET}', 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
}

/// A charge line differing from the base only in the columns given.
///
/// `region` used to keep each case on a scope key of its own; the region is an
/// axis of the *market* now, so the discriminator is `dimension_key` instead —
/// which is an axis of the line and does the same job one table over.
fn insert_line_row(line_id: &str, discriminator: &str, columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO pricing_charge_line (
            tenant_id, charge_line_id, plan_id, phase, charge_kind, sku_id,
            dimension_key{columns})
         VALUES ('{TENANT}', '{line_id}', '{PLAN}', '{PHASE}', 'usage', '{SKU}',
            '{discriminator}'{values})"
    )
}

/// A draft version of an arbitrary line, for the cases that need one per line.
fn version_on(version_id: &str, line_id: &str, revision: u32) -> String {
    format!(
        "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, charge_line_id, \
         plan_revision, lifecycle_state, created_by, created_at_utc, row_version) \
         VALUES ('{TENANT}', '{version_id}', '{line_id}', {revision}, 'draft', '{ACTOR}', \
         '2026-08-02 10:00:00 +00:00', 0)"
    )
}

/// A draft price row naming its line, version and market explicitly.
fn price_on(
    price_id: &str,
    line_id: &str,
    version_id: &str,
    market_id: &str,
    columns: &str,
    values: &str,
) -> String {
    format!(
        "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, \
         charge_line_id, line_version_id, market_price_id, lifecycle_state, created_by, \
         created_at_utc{columns}) \
         VALUES ('{price_id}', '{TENANT}', '{PLAN}', 0, '{line_id}', '{version_id}', \
         '{market_id}', 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00'{values})"
    )
}

/// A draft version of [`CHARGE_LINE`] carrying a chosen meter literal.
fn meter_version(version_id: &str, revision: u32, meter: &str) -> String {
    format!(
        "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, charge_line_id, \
         plan_revision, lifecycle_state, meter, created_by, created_at_utc, row_version) \
         VALUES ('{TENANT}', '{version_id}', '{CHARGE_LINE}', {revision}, 'draft', {meter}, \
         '{ACTOR}', '2026-08-02 10:00:00 +00:00', 0)"
    )
}

/// An INSERT of a **market** on the base line, so a case can vary currency or
/// region without touching the line.
fn insert_market(market_id: &str, line_id: &str, currency: &str, region: &str) -> String {
    format!(
        "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
         currency, region) \
         VALUES ('{TENANT}', '{market_id}', '{line_id}', '{currency}', '{region}')"
    )
}

/// An INSERT whose caller-chosen token lands on whichever table owns it.
///
/// `lifecycle_state` is the price row's; `price_overlay`, `price_eligibility`
/// and `charge_kind` are axes of the charge line. Each gets a row of its own —
/// a line discriminated by `dimension_key`, or a price row on its own market —
/// because three of the four are key columns, so an UPDATE would move the row's
/// key rather than its content.
///
/// `cohort` follows `price_eligibility` because the biconditional binds them: a
/// helper that wrote `none` under `existing_grandfathered` would have every
/// positive case for that token refused by a constraint that is not the one
/// under test.
fn insert_token_row(seq: usize, column: &str, token: &str) -> String {
    let chosen = |name: &str, default: &str| -> String {
        if name == column { token } else { default }.to_owned()
    };
    if column == "lifecycle_state" {
        // A market of its own per row: `uq_pricing_price_market_draft` admits one
        // *draft* per market, and `draft` is one of the tokens under test.
        return format!(
            "INSERT INTO pricing_price (price_id, tenant_id, plan_id, plan_revision, \
             charge_line_id, line_version_id, market_price_id, lifecycle_state, created_by, \
             created_at_utc) \
             VALUES ('cccc0000-0000-0000-0000-{seq:012}', '{TENANT}', '{PLAN}', 0, \
             '{CHARGE_LINE}', '{SEED_VERSION}', 'cccc0000-0000-0000-1111-{seq:012}', \
             '{token}', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        );
    }
    let eligibility = chosen("price_eligibility", "all_subscriptions");
    let cohort = if eligibility == "existing_grandfathered" {
        "1780000000000"
    } else {
        "none"
    };
    format!(
        "INSERT INTO pricing_charge_line (
            tenant_id, charge_line_id, plan_id, phase, price_overlay,
            price_eligibility, cohort, charge_kind, sku_id, dimension_key)
         VALUES ('{TENANT}', 'cccc0000-0000-0000-0000-{seq:012}', '{PLAN}', '{PHASE}',
            '{}', '{eligibility}', '{cohort}', '{}', '{SKU}', 'D{seq}')",
        chosen("price_overlay", "base"),
        chosen("charge_kind", "usage"),
    )
}

/// A line as `uq_pricing_charge_line_logical_scope` sees it: the axes a case
/// varies, plus the two that left the key entirely.
///
/// `region` and `meter` are both off the logical key now — the first to
/// `pricing_market_price`, the second to the line's version — so a case that
/// varies either is asserting that they do **not** discriminate, which is the
/// inverse of what the old fixture asserted and is why they stay in the struct.
#[derive(Clone, Copy)]
struct Line {
    seq: usize,
    charge_kind: &'static str,
    cohort: &'static str,
    dimension_key: &'static str,
}

/// The undimensioned usage line every case below varies one axis of.
const LINE: Line = Line {
    seq: 0,
    charge_kind: "usage",
    cohort: "none",
    dimension_key: "",
};

/// The INSERT for one [`Line`].
///
/// `price_eligibility` follows `cohort` instead of being an axis of its own,
/// for the reason [`insert_token_row`] pairs them: the biconditional binds the
/// two, so a case that set a generation under `all_subscriptions` would be
/// refused by a `CHECK` rather than by the index under test.
fn insert_line(line: &Line) -> String {
    let Line {
        seq,
        charge_kind,
        cohort,
        dimension_key,
    } = *line;
    let eligibility = if cohort == "none" {
        "all_subscriptions"
    } else {
        "existing_grandfathered"
    };
    format!(
        "INSERT INTO pricing_charge_line (
            tenant_id, charge_line_id, plan_id, phase, price_eligibility, cohort,
            charge_kind, sku_id, dimension_key)
         VALUES ('{TENANT}', 'eeee0000-0000-0000-0000-{seq:012}', '{PLAN}', '{PHASE}',
            '{eligibility}', '{cohort}', '{charge_kind}', '{SKU}', '{dimension_key}')"
    )
}

#[tokio::test]
async fn package_block_fields_need_the_kind_that_gives_them_meaning() {
    let conn = migrated_db().await;
    ensure_line(&conn).await;

    // **One rule, now enforced in two places, because its two columns have two
    // owners.** `chk_pricing_price_package_fields_kind` read "a block field
    // requires `model_kind = 'package'`" over two columns of one row. The block
    // *size* is shared geometry and moved to `pricing_charge_line_version` with
    // the kind, so that half is still a `CHECK`. The block *price* is market
    // money and stayed on `pricing_price`, where the kind is one table away and
    // no `CHECK` can reach it — so that half is a trigger. Both halves are
    // proved here, or the split would have quietly dropped one of them.

    // The size half. A kindless version first: `model_kind` is nullable — a
    // draft may be authored before its kind is — so this row's block would once
    // have landed, because `FALSE OR NULL` is NULL and a NULL CHECK result is
    // satisfied.
    must_violate(
        &conn,
        &insert_version(
            "aaaa0001-0000-0000-0000-000000000001",
            "NULL",
            ", package_size",
            ", 100",
        ),
        "chk_pricing_charge_line_version_package_fields_kind",
    )
    .await;
    // And a block on a kind whose money lives somewhere else.
    must_violate(
        &conn,
        &insert_version(
            "aaaa0001-0000-0000-0000-000000000002",
            "'graduated'",
            ", package_size",
            ", 100",
        ),
        "chk_pricing_charge_line_version_package_fields_kind",
    )
    .await;
    // The positive control, without which both of the above would pass against a
    // constraint that refused every row.
    must_succeed(
        &conn,
        &insert_version(
            "aaaa0001-0000-0000-0000-000000000003",
            "'package'",
            ", package_size",
            ", 100",
        ),
    )
    .await;
    // As is a kindless version carrying no block at all: the rule forbids the
    // pairing, not the absent kind.
    must_succeed(
        &conn,
        &insert_version("aaaa0001-0000-0000-0000-000000000004", "NULL", "", ""),
    )
    .await;

    // The money half, against the version the size half just proved. A block
    // price on the `package` version lands; the same price on the kindless one
    // is refused, and by the trigger's own sentence rather than by a `CHECK`
    // that no longer exists.
    must_succeed(
        &conn,
        &insert_price_on(
            "bbbb0001-0000-0000-0000-000000000001",
            "aaaa0001-0000-0000-0000-000000000003",
            "pk",
            ", package_price_minor",
            ", 5000",
        ),
    )
    .await;
    must_abort(
        &conn,
        &insert_price_on(
            "bbbb0001-0000-0000-0000-000000000002",
            "aaaa0001-0000-0000-0000-000000000004",
            "nk",
            ", package_price_minor",
            ", 5000",
        ),
        "package_price_minor is permitted only on a package line version",
    )
    .await;

    let versions = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_charge_line_version",
    )
    .await;
    assert_eq!(versions, "2", "only the two permitted versions landed");
    let rows = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(rows, "1", "and only the block price on the package version");
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
        (
            "price_overlay",
            "chk_pricing_charge_line_overlay",
            &["base"],
        ),
        (
            "price_eligibility",
            "chk_pricing_charge_line_eligibility",
            &[
                "all_subscriptions",
                "new_subscriptions_only",
                "existing_grandfathered",
            ],
        ),
        (
            "charge_kind",
            "chk_pricing_charge_line_charge_kind",
            &["recurring", "usage", "one_time"],
        ),
    ];

    let conn = migrated_db().await;
    seed(&conn).await;
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
            if column == "lifecycle_state" {
                must_succeed(
                    &conn,
                    &insert_market(
                        &format!("cccc0000-0000-0000-1111-{seq:012}"),
                        CHARGE_LINE,
                        "USD",
                        &format!("R{seq}"),
                    ),
                )
                .await;
            }
            must_succeed(&conn, &insert_token_row(seq, column, token)).await;
        }
        // And the token no enum renders, which is the refusal the repository's
        // `CorruptRow` reading rests on. It is a claim about the schema, not
        // about the repository, so only a statement the repository never issues
        // can make it.
        seq += 1;
        if column == "lifecycle_state" {
            must_succeed(
                &conn,
                &insert_market(
                    &format!("cccc0000-0000-0000-1111-{seq:012}"),
                    CHARGE_LINE,
                    "USD",
                    &format!("R{seq}"),
                ),
            )
            .await;
        }
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
    must_succeed(
        &conn,
        &insert_market(
            &format!("cccc0000-0000-0000-1111-{seq:012}"),
            CHARGE_LINE,
            "USD",
            &format!("R{seq}"),
        ),
    )
    .await;
    must_violate(
        &conn,
        &insert_token_row(seq, "lifecycle_state", "retired"),
        "chk_pricing_price_lifecycle_state",
    )
    .await;

    // A CHECK that refused a statement which took effect anyway would be
    // indistinguishable above from one that worked.
    // **Counted across both owners.** Three of the four columns are axes of the
    // charge line and one is the price row's, so the rows this case landed are
    // split between two tables — and the seed's own line, version and row are
    // there too, which is why the base is subtracted rather than assumed away.
    let lines_landed: u64 = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_charge_line",
    )
    .await
    .parse()
    .expect("a count");
    let rows_landed: u64 = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await
    .parse()
    .expect("a count");
    assert_eq!(
        lines_landed + rows_landed - 2,
        landed,
        "exactly the legal rows landed, the seed's line and row aside"
    );
}

#[tokio::test]
async fn each_nullable_token_column_holds_only_the_tokens_its_enum_renders() {
    /// Column, the constraint that guards it, and every token the domain enum
    /// behind it renders.
    ///
    /// **All eight are the line version's now.** They are shared calculation
    /// structure - the model, the meter's folding windows, the quantity source -
    /// so they moved off `pricing_price` with the rest of it, and their `CHECK`s
    /// moved with them rather than being restated.
    const COLUMNS: [(&str, &str, &[&str]); 8] = [
        (
            "model_kind",
            "chk_pricing_charge_line_version_model_kind",
            &["flat", "per_unit", "graduated", "volume", "package"],
        ),
        (
            "billing_timing",
            "chk_pricing_charge_line_version_billing_timing",
            &["advance", "arrears"],
        ),
        (
            "quantity_source",
            "chk_pricing_charge_line_version_quantity_source",
            &["subscription_seat_count", "manual"],
        ),
        (
            "billing_granularity",
            "chk_pricing_charge_line_version_billing_granularity",
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
            "chk_pricing_charge_line_version_aggregation_function",
            &["sum", "peak", "time_weighted"],
        ),
        (
            "aggregation_granularity",
            "chk_pricing_charge_line_version_aggregation_granularity",
            &["hour", "day"],
        ),
        (
            "tier_aggregation_window",
            "chk_pricing_charge_line_version_tier_aggregation_window",
            &[
                "calendar_month",
                "invoice_period",
                "subscription_lifetime",
                "per_event",
                // D-313. The token no other test writes to a database: the five
                // sites that name it are rule tests over in-memory rows, so a
                // CHECK short by this one would refuse an hourly fold at the
                // store with every suite green.
                "per_hour",
            ],
        ),
        (
            "tier_qualification_window",
            "chk_pricing_charge_line_version_tier_qualification_window",
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
                &format!("UPDATE pricing_charge_line_version SET {column} = '{token}' WHERE line_version_id = '{SEED_VERSION}'"),
            )
            .await;
        }
        must_succeed(
            &conn,
            &format!("UPDATE pricing_charge_line_version SET {column} = NULL WHERE line_version_id = '{SEED_VERSION}'"),
        )
        .await;
        must_violate(
            &conn,
            &format!(
                "UPDATE pricing_charge_line_version SET {column} = '{FOREIGN_TOKEN}' WHERE line_version_id = '{SEED_VERSION}'"
            ),
            constraint,
        )
        .await;
        let stored = scalar(
            &conn,
            &format!(
                "SELECT coalesce({column}, 'null') AS v FROM pricing_charge_line_version \
                 WHERE line_version_id = '{SEED_VERSION}'"
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
    /// **The table is part of the case now.** Four of the nine are quantities —
    /// shared calculation structure — and moved to `pricing_charge_line_version`
    /// with their `CHECK`s; the five that are money stayed on `pricing_price`.
    /// A bound proved against the wrong table would be proved against a column
    /// that is not there, which `no such column` reports rather than a passing
    /// constraint.
    const BOUNDS: [(&str, &str, &str, &str, &str); 9] = [
        (
            "pricing_price",
            "amount_minor",
            "chk_pricing_price_amount_non_negative",
            "0",
            "-1",
        ),
        (
            "pricing_charge_line_version",
            "manual_quantity",
            "chk_pricing_charge_line_version_manual_quantity",
            "0",
            "-1",
        ),
        (
            "pricing_charge_line_version",
            "max_hold_granules",
            "chk_pricing_charge_line_version_max_hold_granules",
            "1",
            "0",
        ),
        (
            "pricing_charge_line_version",
            "min_qty_purchase",
            "chk_pricing_charge_line_version_min_qty_purchase",
            "0",
            "-1",
        ),
        (
            "pricing_charge_line_version",
            "min_qty_usage",
            "chk_pricing_charge_line_version_min_qty_usage",
            "0",
            "-1",
        ),
        (
            "pricing_price",
            "package_price_minor",
            "chk_pricing_price_package_price",
            "0",
            "-1",
        ),
        (
            "pricing_charge_line_version",
            "package_size",
            "chk_pricing_charge_line_version_package_size",
            "1",
            "0",
        ),
        (
            "pricing_price",
            "reserved_rate_nano",
            "chk_pricing_price_reserved_rate_nano",
            "0",
            "-1",
        ),
        (
            "pricing_price",
            "unit_rate_nano",
            "chk_pricing_price_unit_rate_nano",
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
        &format!("UPDATE pricing_charge_line_version SET model_kind = 'package' WHERE line_version_id = '{SEED_VERSION}'"),
    )
    .await;

    for (table, column, constraint, admitted, refused) in BOUNDS {
        let row = if table == "pricing_price" {
            format!("price_id = '{SEED}'")
        } else {
            format!("line_version_id = '{SEED_VERSION}'")
        };
        must_succeed(
            &conn,
            &format!("UPDATE {table} SET {column} = {admitted} WHERE {row}"),
        )
        .await;
        must_violate(
            &conn,
            &format!("UPDATE {table} SET {column} = {refused} WHERE {row}"),
            constraint,
        )
        .await;
        // The refused value may not have landed, and absent is a state of every
        // one of these columns: each is required only for some kinds.
        let stored = scalar(
            &conn,
            &format!("SELECT CAST({column} AS TEXT) AS v FROM {table} WHERE {row}"),
        )
        .await;
        assert_eq!(
            stored, admitted,
            "the refused UPDATE of {column} may not land"
        );
        must_succeed(
            &conn,
            &format!("UPDATE {table} SET {column} = NULL WHERE {row}"),
        )
        .await;
    }
}

#[tokio::test]
async fn the_entity_tag_refuses_the_value_just_past_its_bound() {
    // `revision` on `pricing_plan` and on `pricing_price_overlay` already
    // carries this guard. `row_version` is the same shape read the same way —
    // through a conversion that answers `CorruptRow` — so without it a poisoned
    // row is one no typed path can read back, while the write that landed the
    // value met no objection.
    //
    // Not in `BOUNDS`: the column is `NOT NULL`, and that loop resets each
    // column through `NULL` between cases.
    let conn = migrated_db().await;
    seed(&conn).await;

    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET row_version = 0 WHERE price_id = '{SEED}'"),
    )
    .await;
    must_violate(
        &conn,
        &format!("UPDATE pricing_price SET row_version = -1 WHERE price_id = '{SEED}'"),
        "chk_pricing_price_row_version",
    )
    .await;
}

#[tokio::test]
async fn the_cohort_pairing_holds_both_ways_and_the_horizon_needs_its_class() {
    let conn = migrated_db().await;

    // **The pair split across two tables, and so did their guards.** The cohort
    // and the eligibility class are axes of the charge line, so their
    // biconditional is still a `CHECK` - one table over. The horizon is market
    // money and stayed on `pricing_price`, where the class it needs is a join
    // away, so that half is a trigger. Both are proved, because a split that
    // dropped one would look exactly like this file passing.

    // Forward direction of the biconditional: a cohort on a line of a class that
    // retains nobody. Such a line sits on a key no resolution class ever selects.
    for (seq, eligibility) in ["all_subscriptions", "new_subscriptions_only"]
        .into_iter()
        .enumerate()
    {
        must_violate(
            &conn,
            &insert_line_row(
                &format!("dddd0000-0000-0000-0000-00000000000{seq}"),
                &format!("F{seq}"),
                ", price_eligibility, cohort",
                &format!(", '{eligibility}', '1780000000000'"),
            ),
            "chk_pricing_charge_line_cohort_eligibility",
        )
        .await;
    }

    // Reverse direction, and the more damaging one: a grandfathered line with no
    // generation lands on the `all_subscriptions` successor's own key and, being
    // immutable, occupies the key the next reprice needs.
    must_violate(
        &conn,
        &insert_line_row(
            "dddd0000-0000-0000-0000-000000000010",
            "F10",
            ", price_eligibility, cohort",
            ", 'existing_grandfathered', 'none'",
        ),
        "chk_pricing_charge_line_cohort_eligibility",
    )
    .await;

    // The horizon is the grandfathered class's alone: it expires a *retained
    // generation*, and the other two classes retain nobody. The lines below are
    // legal - they carry `cohort = 'none'` under a non-grandfathered class - so
    // the only thing that can refuse the price row is the guard under test.
    for (seq, eligibility) in ["all_subscriptions", "new_subscriptions_only"]
        .into_iter()
        .enumerate()
    {
        let line = format!("dddd0000-0000-0000-0000-00000000002{seq}");
        let market = format!("dddd0000-0000-0000-0000-00000000012{seq}");
        let version = format!("dddd0000-0000-0000-0000-00000000022{seq}");
        must_succeed(
            &conn,
            &insert_line_row(
                &line,
                &format!("G{seq}"),
                ", price_eligibility",
                &format!(", '{eligibility}'"),
            ),
        )
        .await;
        must_succeed(&conn, &insert_market(&market, &line, "USD", "EU")).await;
        must_succeed(&conn, &version_on(&version, &line, 0)).await;
        must_abort(
            &conn,
            &price_on(
                &format!("dddd0000-0000-0000-0000-00000000032{seq}"),
                &line,
                &version,
                &market,
                ", grandfather_until",
                ", '2027-01-01 00:00:00 +00:00'",
            ),
            "grandfather_until is permitted only on an existing_grandfathered line",
        )
        .await;
    }

    // The positive controls, without which every case above would pass against
    // guards that refused every row: the grandfathered class carries both a
    // generation and a horizon, and the default class carries neither.
    let kept = "dddd0000-0000-0000-0000-000000000030";
    must_succeed(
        &conn,
        &insert_line_row(
            kept,
            "H0",
            ", price_eligibility, cohort",
            ", 'existing_grandfathered', '1780000000000'",
        ),
    )
    .await;
    must_succeed(
        &conn,
        &insert_market("dddd0000-0000-0000-0000-000000000130", kept, "USD", "EU"),
    )
    .await;
    must_succeed(
        &conn,
        &version_on("dddd0000-0000-0000-0000-000000000230", kept, 0),
    )
    .await;
    must_succeed(
        &conn,
        &price_on(
            "dddd0000-0000-0000-0000-000000000330",
            kept,
            "dddd0000-0000-0000-0000-000000000230",
            "dddd0000-0000-0000-0000-000000000130",
            ", grandfather_until",
            ", '2027-01-01 00:00:00 +00:00'",
        ),
    )
    .await;
    must_succeed(
        &conn,
        &insert_line_row(
            "dddd0000-0000-0000-0000-000000000031",
            "H1",
            ", price_eligibility",
            ", 'new_subscriptions_only'",
        ),
    )
    .await;

    let lines_landed = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_charge_line",
    )
    .await;
    assert_eq!(lines_landed, "4", "only the four legal lines landed");
    let rows = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(rows, "1", "and only the horizon on the class that retains");
}

/// D-372: two units of one SKU are **one** key, and the index that used to make
/// them two markets is gone.
///
/// This case replaces `a_scope_key_slice_prices_a_meter_line_once_dimensioned_or_not`,
/// whose subject was `uq_pricing_price_meter_line_current`. That index keyed the
/// usage line without `charge_kind`, so it refused a pair the scope-key index
/// admitted, and the pair was how the case identified which index answered. D-372
/// drops the index and puts `sku_id` on the key in the meter's place, so the same
/// two rows are now told apart by their SKU and by nothing else.
#[tokio::test]
async fn two_units_of_one_sku_are_one_key_and_two_skus_are_two() {
    let conn = migrated_db().await;

    must_succeed(&conn, &insert_line(&LINE)).await;

    // Two usage lines on one slice, differing in nothing the key admits. Under
    // D-196 a differing meter made them two keys; under D-372 the meter is not
    // an axis at all - it is content of the line's version - so they are one
    // key, and `uq_pricing_charge_line_logical_scope` is what says so.
    let err = exec(&conn, &insert_line(&Line { seq: 1, ..LINE }))
        .await
        .expect_err("a second unit of one SKU is the same key");
    let message = err.to_string();
    assert!(
        message.contains("UNIQUE constraint failed")
            && message.contains("pricing_charge_line.sku_id"),
        "the refusal must come from the logical-scope index, over an axis list carrying the \
         SKU: {message}"
    );

    // And a line that disagrees about `charge_kind` is still a different key:
    // the axes that remain in the key still discriminate.
    must_succeed(
        &conn,
        &insert_line(&Line {
            seq: 2,
            charge_kind: "recurring",
            ..LINE
        }),
    )
    .await;

    // As does the tenth axis, which stayed in the key when the meter left it.
    must_succeed(
        &conn,
        &insert_line(&Line {
            seq: 3,
            dimension_key: "region=eu",
            ..LINE
        }),
    )
    .await;

    let landed = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_charge_line",
    )
    .await;
    assert_eq!(
        landed, "3",
        "the three distinct keys landed and the duplicate did not"
    );
}

#[tokio::test]
async fn neither_free_form_key_axis_admits_the_separator() {
    let conn = migrated_db().await;
    ensure_line(&conn).await;

    // **Two axes, two tables.** The region is an axis of the market and the
    // meter is content of the line version, so the pair that used to share one
    // `CHECK` per column on `pricing_price` is now one `CHECK` on each of two
    // tables. Both still exist, which is the whole of this case: a separator in
    // either renders the same canonical key string as a different key.
    must_violate(
        &conn,
        &insert_market(
            "aaaa0091-0000-0000-0000-000000000001",
            CHARGE_LINE,
            "USD",
            "eu|west",
        ),
        "chk_pricing_market_price_region_no_separator",
    )
    .await;
    must_violate(
        &conn,
        &meter_version("aaaa0091-0000-0000-0000-000000000002", 7, "'api|calls'"),
        "chk_pricing_charge_line_version_meter_no_separator",
    )
    .await;

    // The positive controls: a separator-free meter lands, and so does no meter
    // at all - the arm the nullable constraint's disjunct exists for.
    must_succeed(
        &conn,
        &meter_version("aaaa0091-0000-0000-0000-000000000003", 8, "'api_calls'"),
    )
    .await;
    must_succeed(
        &conn,
        &meter_version("aaaa0091-0000-0000-0000-000000000004", 9, "NULL"),
    )
    .await;
    must_succeed(
        &conn,
        &insert_market(
            "aaaa0091-0000-0000-0000-000000000005",
            CHARGE_LINE,
            "USD",
            "us",
        ),
    )
    .await;
}
