//! The append-only and frozen guards of `products_audit_log` and
//! `products_entity_version`, **executed on Postgres**.
//!
//! `postgres_head_guards` covers the two head tables; these are the other two
//! guarded tables, and they guard a different thing. A head row is *mutable
//! under rules*; these two are records — one an append-only trail with a single
//! sanctioned transition, the other frozen outright.
//!
//! # The audit trail's guard is an allow-list, and that is the whole risk
//!
//! `bss.products_audit_log_append_only()` refuses every `DELETE`, then admits an
//! `UPDATE` **only** when it is precisely the sealing transition: `unsealed ->
//! sealed`, all three of `chain_id`/`seq`/`row_hash` present, and every one of
//! the thirteen content columns unchanged. Anything else falls through to the
//! `RAISE`.
//!
//! An allow-list fails in the dangerous direction when it is too *wide*: one
//! forgotten `IS NOT DISTINCT FROM` and that column becomes editable, forever,
//! in the one table the design says a wrong value cannot be repaired in. A
//! reading comparison cannot see a missing conjunct — there is nothing there to
//! read. Executing the write can, and this suite walks **every** content column
//! rather than sampling, because sampling is how a missing conjunct survives.
//!
//! # Why raw SQL
//!
//! The claim is that the database refuses, whatever reaches it. A probe routed
//! through `infra::storage::repo` would pass with the trigger dropped, because
//! the repository never forms these statements.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_frozen_guards -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-audit-table:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const TENANT: &str = "00000000-0000-0000-0000-000000007e42";
const AUDIT: &str = "00000000-0000-0000-0000-0000000000a1";
const ACTOR: &str = "00000000-0000-0000-0000-00000000ac70";
const SUBJECT: &str = "00000000-0000-0000-0000-000000001111";
const CHAIN: &str = "00000000-0000-0000-0000-0000000000c1";

/// Every content column the sealing allow-list pins, paired with a value
/// different from the seed's.
///
/// **Transcribed from the table, not from the trigger.** A list derived from the
/// guard's own conjuncts could only prove the guard equals itself; this one is
/// the column roster the table declares, minus the **five** the sealing
/// transition is *supposed* to move: `seal_state`, `chain_id`, `seq`,
/// `row_hash` and `prev_hash`. The arithmetic is 18 declared minus 13 pinned
/// here; an earlier revision of this sentence said four and left `prev_hash`
/// unnamed, which made the count look like it balanced when it did not.
///
/// `prev_hash` is excluded for the same reason as the other four and needs no
/// separate guard: the trigger admits exactly **one** update per row
/// (`OLD.seal_state = 'unsealed' AND NEW.seal_state = 'sealed'`), so a sealed
/// row admits no second update in which the column could be moved again.
///
/// A fourteenth content column added later and forgotten in the allow-list
/// shows up here as an admitted write.
const CONTENT_COLUMNS: &[(&str, &str)] = &[
    // The primary key is pinned by the same clause as the rest, and is
    // tamperable in the same statement: the `WHERE` still finds the row by its
    // old id while `SET` moves it.
    ("audit_id", "'00000000-0000-0000-0000-0000000077fe'"),
    ("tenant_id", "'00000000-0000-0000-0000-0000000077ff'"),
    ("actor_ref", "'00000000-0000-0000-0000-0000000000ff'"),
    ("action", "'tampered'"),
    ("subject_kind", "'tampered'"),
    ("subject_id", "'00000000-0000-0000-0000-0000000000fe'"),
    ("subject_revision", "99"),
    ("error_code", "'TAMPERED'"),
    ("attempted_key", "'tampered'"),
    ("reason", "'tampered'"),
    ("correlation_id", "'00000000-0000-0000-0000-0000000000fd'"),
    ("written_at", "now() + interval '1 day'"),
    ("session_id", "'00000000-0000-0000-0000-0000000000fc'"),
];

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

async fn admitted(conn: &DatabaseConnection, sql: &str) {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .unwrap_or_else(|e| panic!("the guard refused a write it must admit:\n{sql}\n{e}"));
}

/// One `unsealed` audit row.
async fn seed_audit(conn: &DatabaseConnection) {
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_audit_log
               (audit_id, tenant_id, actor_ref, action, subject_kind, subject_id,
                subject_revision, error_code, attempted_key, reason, correlation_id,
                written_at, session_id, seal_state, chain_id, seq, prev_hash, row_hash)
             VALUES ('{AUDIT}', '{TENANT}', '{ACTOR}', 'create', 'product', '{SUBJECT}',
                1, NULL, NULL, 'seeded', NULL, now(), NULL, 'unsealed', NULL, NULL, NULL, NULL)"
        ),
    )
    .await;
}

/// One frozen version row.
async fn seed_version(conn: &DatabaseConnection) {
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_entity_version
               (tenant_id, entity_kind, entity_id, published_version, content,
                content_digest, digest_version, approval_ref, actor_ref, published_at)
             VALUES ('{TENANT}', 'product', '{SUBJECT}', 1, '{{}}', '\\x00'::bytea, 1,
                NULL, '{ACTOR}', now())"
        ),
    )
    .await;
}

/// **The audit trail admits no `DELETE`.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_audit_trail_admits_no_delete() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_audit(&conn).await;

    let message = refusal(
        &conn,
        &format!("DELETE FROM bss.products_audit_log WHERE audit_id = '{AUDIT}'"),
    )
    .await;
    assert!(
        message.contains("products_audit_log is append-only: DELETE is not permitted"),
        "the DELETE arm must answer, not the generic tail: {message}"
    );
}

/// **The sealing transition is admitted, exactly as written.**
///
/// Asserted first among the update cases, and deliberately: every refusal below
/// is only meaningful if the *permitted* write actually goes through. A guard
/// that refused everything would satisfy all the negative cases and would have
/// closed the sealing seam the design reserves.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_sealing_transition_is_admitted() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_audit(&conn).await;

    admitted(
        &conn,
        &format!(
            "UPDATE bss.products_audit_log
                SET seal_state = 'sealed', chain_id = '{CHAIN}', seq = 0,
                    row_hash = '\\x01'::bytea
              WHERE audit_id = '{AUDIT}'"
        ),
    )
    .await;
}

/// **Every content column is pinned across the sealing transition** — all
/// thirteen, walked rather than sampled.
///
/// Each case seals *and* tampers with one column in the same statement, which is
/// the only shape the allow-list could plausibly leak through: a tamper on its
/// own is refused by the transition conjunct above it, so it would prove nothing
/// about the per-column pins.
///
/// **One row per column, one database for the lot.** The rows are addressed by
/// their own primary keys, so they are already independent; and they have to be
/// distinct rows rather than one row reused, because the guard forbids `DELETE`
/// and a sealed row cannot be returned to `unsealed` for the next column. That
/// constraint is the table's whole point, so the suite works with it rather than
/// around it.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn no_content_column_may_move_under_the_sealing_transition() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    for (index, (column, tampered)) in CONTENT_COLUMNS.iter().enumerate() {
        let row = format!("00000000-0000-0000-0000-0000000001{index:02x}");
        admitted(
            &conn,
            &format!(
                "INSERT INTO bss.products_audit_log
                   (audit_id, tenant_id, actor_ref, action, subject_kind, subject_id,
                    subject_revision, error_code, attempted_key, reason, correlation_id,
                    written_at, session_id, seal_state, chain_id, seq, prev_hash, row_hash)
                 VALUES ('{row}', '{TENANT}', '{ACTOR}', 'create', 'product', '{SUBJECT}',
                    1, NULL, NULL, 'seeded', NULL, now(), NULL, 'unsealed', NULL, NULL, NULL, NULL)"
            ),
        )
        .await;

        let message = refusal(
            &conn,
            &format!(
                "UPDATE bss.products_audit_log
                    SET seal_state = 'sealed', chain_id = '{CHAIN}', seq = 0,
                        row_hash = '\\x01'::bytea, {column} = {tampered}
                  WHERE audit_id = '{row}'"
            ),
        )
        .await;
        assert!(
            message.contains("products_audit_log is append-only"),
            "{column} moved under the sealing transition and the guard did not refuse: {message}"
        );
    }
}

/// **An `unsealed -> unsealed` update is refused**, even when it changes
/// nothing the allow-list names.
///
/// The allow-list's first conjunct is the transition itself. A guard that tested
/// only the per-column pins would admit this and leave the trail editable in
/// place.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn an_update_that_is_not_the_sealing_transition_is_refused() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_audit(&conn).await;

    let message = refusal(
        &conn,
        &format!("UPDATE bss.products_audit_log SET reason = 'edited' WHERE audit_id = '{AUDIT}'"),
    )
    .await;
    assert!(
        message.contains("products_audit_log is append-only"),
        "an ordinary edit must be refused: {message}"
    );
}

/// **A seal missing any of its three columns is refused.**
///
/// The `CHECK` constraint would also catch a `sealed` row with a NULL
/// `chain_id`, and that is the point of asserting it here: the two layers are
/// **not** redundant. The trigger's conjunct is what refuses the transition; if
/// it were dropped, the `CHECK` would still refuse this exact statement and the
/// suite would stay green while the trigger's arm had gone. So the message is
/// asserted, not merely the failure.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_seal_missing_its_columns_is_refused_by_the_trigger() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_audit(&conn).await;

    let message = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_audit_log SET seal_state = 'sealed', chain_id = '{CHAIN}', \
             seq = 0 WHERE audit_id = '{AUDIT}'"
        ),
    )
    .await;
    assert!(
        message.contains("products_audit_log is append-only"),
        "the trigger's own arm must be what refuses a seal with no row_hash, not only the \
         CHECK beneath it: {message}"
    );
}

/// **A frozen version row admits neither `UPDATE` nor `DELETE`**, and each is
/// refused by its own arm.
///
/// The two messages are asserted apart because the function's `DELETE` arm is
/// its fall-through: a body that lost its `UPDATE` branch would still refuse an
/// update, with the delete message. Same outcome, different guard, and only the
/// text tells them apart.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_frozen_version_row_admits_neither_update_nor_delete() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_version(&conn).await;

    let update = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_entity_version SET content = 'tampered' \
             WHERE entity_id = '{SUBJECT}'"
        ),
    )
    .await;
    assert!(
        update.contains("frozen: UPDATE is not permitted"),
        "the UPDATE arm must answer, not the delete fall-through: {update}"
    );

    let delete = refusal(
        &conn,
        &format!("DELETE FROM bss.products_entity_version WHERE entity_id = '{SUBJECT}'"),
    )
    .await;
    assert!(
        delete.contains("frozen: DELETE is not permitted"),
        "the DELETE arm must answer with its own message: {delete}"
    );
}
