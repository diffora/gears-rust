//! The head-row guards, **executed on Postgres** — the engine that serves them.
//!
//! # The hole this closes
//!
//! The two engine arms of these guards are not merely different SQL; they are
//! different *shapes*. `SQLite` gets a separate trigger per clause —
//! `no_delete`, `immutable_columns`, `internal_revision`, `published_version`,
//! `published_version_row`, `published_version_terminal`, `lifecycle_edge`,
//! `bucket_i`, `bucket_iii`, and on the SKU table `composition_pending` as a
//! tenth. Postgres gets **one** trigger calling **one** `plpgsql` function whose
//! body carries all of them as sequential `IF` blocks.
//!
//! `migrations_tests` executes the `SQLite` arm. Until this file, nothing
//! executed the Postgres one: the two were compared clause for clause by
//! *reading*, and a clause that differed, was mis-ordered, or was silently
//! absent from the function body would have been invisible. That is precisely
//! how the Phase 6 `json`-column defect reached the tree — correct on the
//! mirror, inoperative on the engine production runs — and how the name index's
//! classifier arm was found resting on an untested disjunct.
//!
//! A count is not the check. Matching trigger counts against `IF`-block counts
//! agrees on a number and says nothing about whether the tenth block tests what
//! the tenth trigger tests. Only executing them does.
//!
//! And the number is not even one number: `products_sku` carries **ten** `IF`
//! blocks, `products_product` **nine** — the tenth is `composition_pending`,
//! which is a SKU-only column (§4.2: `bundle` is a value of the SKU-only
//! `type`), so the twin has nothing to match it. A doc that quoted a single
//! "ten and ten" for both tables was describing one of them.
//!
//! # Why every write here goes past the repository
//!
//! Deliberately raw SQL. The claim under test is that the **database** refuses,
//! whatever reaches it — "the guard judges the data, never the door". A probe
//! that went through `infra::storage::repo` would pass with every guard dropped,
//! because the repository would never have formed the forbidden statement.
//!
//! Seeding is raw for the same reason and one more: an `INSERT` is not guarded,
//! so a row can be placed in states no admitted sequence of updates could reach
//! — a `retired` head at `published_version = 1`, for instance — which is what
//! lets the later clauses be reached at all.
//!
//! # The clauses are ordered, and the order is load-bearing
//!
//! The function returns on its first failing `IF`. Reaching the terminal-head
//! clause therefore means *satisfying* the frozen-row clause above it, which is
//! why those cases seed a `products_entity_version` row they otherwise would not
//! need. A case that forgot this would still be red — but for the clause above
//! the one it names, which is a green-looking suite pointed at the wrong guard.
//! Every assertion below names the message it expects.
//!
//! # Both tables in every case
//!
//! The Product and SKU guards are twins, and twins sliced apart in Phase 6
//! diverged six times. Each case here asserts both, so a clause repaired on one
//! table and forgotten on the other cannot pass.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_head_guards -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-append-only-guard:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const TENANT: &str = "00000000-0000-0000-0000-000000007e42";
const BRAND: &str = "00000000-0000-0000-0000-00000000b2a0";
const PRODUCT: &str = "00000000-0000-0000-0000-000000001111";
const SKU: &str = "00000000-0000-0000-0000-000000002222";
const ACTOR: &str = "00000000-0000-0000-0000-00000000ac70";

/// Run one statement, expecting the guard to refuse it, and hand back the
/// engine's message.
///
/// A statement that **succeeds** is the failure this returns on: a guard that
/// stopped refusing is exactly what these cases exist to catch, and it would
/// otherwise show up only as a confusing assertion about an empty string.
async fn refusal(conn: &DatabaseConnection, sql: &str) -> String {
    match conn
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
    {
        Ok(_) => panic!("the guard admitted a write it must refuse:\n{sql}"),
        Err(e) => e.to_string(),
    }
}

/// Run one statement that the guards must **admit**.
///
/// Present in the cases that need a legitimate write beside the refused one:
/// a suite that only ever asserts refusals would pass against a trigger that
/// refuses everything, which is not the contract either.
async fn admitted(conn: &DatabaseConnection, sql: &str) {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .unwrap_or_else(|e| panic!("the guard refused a write it must admit:\n{sql}\n{e}"));
}

/// A `draft` Product and its `draft` SKU, both at `internal_revision = 1`,
/// `published_version = 0`.
///
/// `lifecycle_state` and `published_version` are parameters because the later
/// clauses can only be reached from states no admitted update sequence
/// produces.
async fn seed(conn: &DatabaseConnection, state: &str, published_version: i64) {
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_product
               (tenant_id, product_id, brand_id, name, name_normalized, product_code,
                lifecycle_state, internal_revision, published_version, region_scope,
                brand_scope, created_by, created_at, updated_at)
             VALUES ('{TENANT}', '{PRODUCT}', '{BRAND}', 'Fibre 500', 'fibre 500', 'FIBRE-500',
                '{state}', 1, {published_version}, 'eu', '', 'principal:a', now(), now())"
        ),
    )
    .await;
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_sku
               (tenant_id, sku_id, product_id, sku_code, lifecycle_state, internal_revision,
                published_version, composition_pending, region_scope, brand_scope, created_by,
                created_at, updated_at)
             VALUES ('{TENANT}', '{SKU}', '{PRODUCT}', 'FIBRE-500-STD', '{state}', 1,
                {published_version}, false, 'eu', '', 'principal:a', now(), now())"
        ),
    )
    .await;
}

/// Freeze `version` for both entity kinds, so a `published_version` bump to it
/// clears the existence clause and the *next* clause is what answers.
async fn freeze(conn: &DatabaseConnection, version: i64) {
    for (kind, id) in [("product", PRODUCT), ("sku", SKU)] {
        admitted(
            conn,
            &format!(
                "INSERT INTO bss.products_entity_version
                   (tenant_id, entity_kind, entity_id, published_version, content,
                    content_digest, digest_version, approval_ref, actor_ref, published_at)
                 VALUES ('{TENANT}', '{kind}', '{id}', {version}, '{{}}',
                    '\\x00'::bytea, 1, NULL, '{ACTOR}', now())"
            ),
        )
        .await;
    }
}

/// **Neither head table admits a `DELETE`.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_delete_is_refused_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    let product = refusal(
        &conn,
        &format!("DELETE FROM bss.products_product WHERE product_id = '{PRODUCT}'"),
    )
    .await;
    assert!(
        product.contains("products_product is append-only"),
        "the Product delete must be refused by its own append-only clause: {product}"
    );

    let sku = refusal(
        &conn,
        &format!("DELETE FROM bss.products_sku WHERE sku_id = '{SKU}'"),
    )
    .await;
    assert!(
        sku.contains("products_sku is append-only"),
        "the SKU delete must be refused by its own append-only clause: {sku}"
    );
}

/// **The row-identity columns are immutable**, `created_at` among them.
///
/// `created_at` is asserted explicitly because it is the column §5's roster
/// omits while the guard refuses it in the *same clause* as `tenant_id` and the
/// primary key. If that clause were ever narrowed to §5's list, this is the
/// assertion that would notice.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_row_identity_columns_are_immutable_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    // `clock_timestamp()`, not `now()`. `now()` is `transaction_timestamp()`,
    // constant for a whole transaction, so it equals the value `seed` wrote
    // whenever seed and probe share one — the guard's
    // `NEW.created_at IS DISTINCT FROM OLD.created_at` would then be false,
    // the clause would not fire, and this case would fail for a reason that
    // has nothing to do with the guard. It passes today only because the two
    // land in separate implicit transactions. `clock_timestamp()` advances
    // within a transaction and is immune to that coupling.
    for column in [
        "created_by = 'principal:b'",
        "created_at = clock_timestamp()",
    ] {
        let product = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_product SET {column}, internal_revision = internal_revision + 1 \
                 WHERE product_id = '{PRODUCT}'"
            ),
        )
        .await;
        assert!(
            product.contains("are immutable"),
            "Product: {column} must be refused as immutable: {product}"
        );

        let sku = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_sku SET {column}, internal_revision = internal_revision + 1 \
                 WHERE sku_id = '{SKU}'"
            ),
        )
        .await;
        assert!(
            sku.contains("are immutable"),
            "SKU: {column} must be refused as immutable: {sku}"
        );
    }
}

/// **`internal_revision` moves by exactly one, or the write is refused.**
///
/// Both directions of "not exactly one" are probed — an unchanged value and a
/// jump of two — because a guard written as `NEW > OLD` would admit the jump
/// and a guard written as `NEW <> OLD` would admit it too.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn internal_revision_must_move_by_exactly_one_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    for revision in ["internal_revision", "internal_revision + 2"] {
        let product = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_product SET name = 'Fibre 600', internal_revision = {revision} \
                 WHERE product_id = '{PRODUCT}'"
            ),
        )
        .await;
        assert!(
            product.contains("internal_revision must move by exactly one"),
            "Product: {revision} must be refused: {product}"
        );

        let sku = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_sku SET region_scope = 'apac', internal_revision = {revision} \
                 WHERE sku_id = '{SKU}'"
            ),
        )
        .await;
        assert!(
            sku.contains("internal_revision must move by exactly one"),
            "SKU: {revision} must be refused: {sku}"
        );
    }

    // And the admitted case, so this is not a suite that would pass against a
    // trigger refusing every update.
    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_product SET name = 'Fibre 600', \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
}

/// **`published_version` moves by `+1` or not at all.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn published_version_moves_by_one_or_not_at_all_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "published", 1).await;

    for target in ["published_version + 2", "published_version - 1"] {
        let product = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_product SET published_version = {target}, \
                 internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
            ),
        )
        .await;
        assert!(
            product.contains("published_version only moves by +1"),
            "Product: {target} must be refused: {product}"
        );

        let sku = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_sku SET published_version = {target}, \
                 internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
            ),
        )
        .await;
        assert!(
            sku.contains("published_version only moves by +1"),
            "SKU: {target} must be refused: {sku}"
        );
    }
}

/// **A `published_version` bump requires its frozen version row to exist
/// already.**
///
/// This is the clause that makes "freeze first, bump second" physical rather
/// than a convention of the publish path.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_published_version_bump_without_its_frozen_row_is_refused_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    let product = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_product SET published_version = 1, \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    assert!(
        product.contains("requires the matching products_entity_version row"),
        "Product: a bump with no frozen row must be refused: {product}"
    );

    let sku = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET published_version = 1, \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        sku.contains("requires the matching products_entity_version row"),
        "SKU: a bump with no frozen row must be refused: {sku}"
    );

    // With the row in place the same statement is admitted — which is what
    // proves the clause tests the row's existence and not the bump itself.
    freeze(&conn, 1).await;
    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_product SET published_version = 1, \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET published_version = 1, \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
}

/// **A terminal head admits no `published_version` bump**, even with the frozen
/// row present.
///
/// The freeze is what makes this case reach its own clause: without it the
/// existence clause above answers first, and the suite would be green about the
/// wrong guard.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_terminal_head_admits_no_published_version_bump_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "retired", 1).await;
    freeze(&conn, 2).await;

    let product = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_product SET published_version = 2, \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    assert!(
        product.contains("not admitted on a terminal head"),
        "Product: the terminal clause must be the one that answers, not the existence clause \
         above it: {product}"
    );

    let sku = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET published_version = 2, \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        sku.contains("not admitted on a terminal head"),
        "SKU: the terminal clause must be the one that answers: {sku}"
    );
}

/// **The lifecycle edge roster is enforced**, and it is the same roster on both
/// tables.
///
/// `draft -> retired` is the probe: it is a plausible-looking edge that the
/// roster does not carry, so a guard that merely checked "the target is a valid
/// state" would admit it and a guard that checked the roster refuses it.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn an_unlisted_lifecycle_edge_is_refused_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    let product = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_product SET lifecycle_state = 'retired', \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    assert!(
        product.contains("is not an admitted edge"),
        "Product: draft -> retired is not on the roster: {product}"
    );

    let sku = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET lifecycle_state = 'retired', \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        sku.contains("is not an admitted edge"),
        "SKU: draft -> retired is not on the roster: {sku}"
    );

    // `draft -> discarded` is on it, and is admitted — the pair is what shows
    // the clause reads a roster rather than refusing every transition.
    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_product SET lifecycle_state = 'discarded', \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
}

/// **Bucket-i columns are admitted only before first publish, on a non-terminal
/// head.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn bucket_i_columns_are_refused_after_first_publish_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "published", 1).await;

    let product = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_product SET product_code = 'FIBRE-600', \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    assert!(
        product.contains("bucket-i columns are admitted only before first publish"),
        "Product: a published head admits no bucket-i write: {product}"
    );

    let sku = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET sku_code = 'FIBRE-600-STD', \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        sku.contains("bucket-i columns are admitted only before first publish"),
        "SKU: a published head admits no bucket-i write: {sku}"
    );
}

/// **Bucket-iii columns are admitted only while the head is non-terminal.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn bucket_iii_columns_are_refused_on_a_terminal_head_on_both_head_tables() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "retired", 1).await;

    let product = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_product SET region_scope = 'apac', \
             internal_revision = internal_revision + 1 WHERE product_id = '{PRODUCT}'"
        ),
    )
    .await;
    assert!(
        product.contains("bucket-iii columns are admitted only while the head is non-terminal"),
        "Product: a retired head admits no bucket-iii write: {product}"
    );

    let sku = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET region_scope = 'apac', \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        sku.contains("bucket-iii columns are admitted only while the head is non-terminal"),
        "SKU: a retired head admits no bucket-iii write: {sku}"
    );
}

/// **`composition_pending` moves only in the same statement as a
/// `published_version` bump** — the SKU table's tenth clause, and the one with
/// no Product twin.
///
/// It is also the clause behind a guard blind spot worth remembering: a
/// same-value save on a terminal head is a bare `internal_revision` bump that
/// **no** `SQLite` trigger sees, because those fire on `NEW IS NOT OLD`. The
/// Postgres arm is written as an `IF` inside one function that runs on every
/// update, so its reach is not identical to its mirror's — which is exactly why
/// reading the two arms against each other was never enough.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn composition_pending_moves_only_with_a_published_version_bump() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "draft", 0).await;

    let alone = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET composition_pending = true, \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
    assert!(
        alone.contains("composition_pending is admitted only in the same statement"),
        "a lone composition_pending write must be refused: {alone}"
    );

    // Riding a bump, it is admitted — the pair is what shows the clause reads
    // the *pairing* rather than freezing the column outright.
    freeze(&conn, 1).await;
    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_sku SET composition_pending = true, published_version = 1, \
             internal_revision = internal_revision + 1 WHERE sku_id = '{SKU}'"
        ),
    )
    .await;
}
