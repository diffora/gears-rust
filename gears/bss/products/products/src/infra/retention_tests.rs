//! The three unattended acts, over a real `SQLite` schema: the retention
//! sweep, the age-triggered tombstone and the restore drill.
//!
//! # The sweep is driven per class, not through the loop
//!
//! `sweep` iterates tenants and classes and swallows per-tenant failures on
//! purpose, so a case driving it whole can only assert what it finds
//! afterwards. [`super::sweep_class`] answers a typed [`ClassOutcome`], which
//! is what the criteria are written against — and one case reads the **audit
//! row** instead, because the `DoD` obliges the pass to be audited with its
//! class, its clock and its verdict, and a typed return proves none of that.
//!
//! # What the storage actually admits at this commit, and why it matters here
//!
//! One table of four. Every case below that expects a **hold** is asserting a
//! guard that ships (`m20260901_000010`, `m20260901_000013`, and the five
//! evidence migrations P-D-136 keeps), and the one that expects a **collect**
//! is asserting `m20260829_000007`'s opened referential predicate. So these
//! are not tests of a policy this module invented — they are tests that the
//! sweep reports what the engine decided.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::sweep_class;
use crate::domain::canonical;
use crate::domain::retention::{RecordClass, RetentionCaps};
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;

const TENANT: Uuid = Uuid::from_u128(0x7e_51);
const SYSTEM: Uuid = Uuid::from_u128(0x5f_51);

struct Harness {
    dsn: String,
    db: DBProvider<DbError>,
    outbox: Arc<Outbox>,
    #[allow(dead_code)]
    _handle: OutboxHandle,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

async fn harness() -> Harness {
    let path = std::env::temp_dir().join(format!(
        "bss-products-retention-gc-{}.sqlite3",
        Uuid::new_v4()
    ));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run this gear's own migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier"),
    )
    .await
    .expect("run the outbox facility's own migrator");
    let handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox pipeline");
    let outbox = Arc::clone(handle.outbox());
    Harness {
        dsn,
        db: DBProvider::<DbError>::new(db),
        outbox,
        _handle: handle,
    }
}

/// Caps with every window at `days` and the drill unconfigured.
fn caps(days: u32) -> RetentionCaps {
    RetentionCaps {
        financial_days: days,
        version_days: days,
        audit_days: days,
        pseudonymization_age_days: days,
        drill_cadence_hours: 24,
        drill_target_dsn: None,
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// An instant `days` in the past, truncated the way every write path does.
fn days_ago(days: i64) -> chrono::DateTime<Utc> {
    canonical::write_instant(Utc::now() - Duration::days(days))
}

/// Freeze one entity version.
async fn seed_entity_version(
    h: &Harness,
    entity_id: Uuid,
    version: i64,
    at: chrono::DateTime<Utc>,
) {
    let conn = h.db.conn().expect("connection");
    let content = format!("{{\"name\":\"v{version}\"}}");
    repo::insert_entity_version(
        &conn,
        &scope(),
        repo::NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: repo::VersionedEntityKind::Product,
            entity_id,
            published_version: version,
            content: content.clone(),
            content_digest: canonical::content_digest(&content),
            digest_version: canonical::DIGEST_VERSION,
            approval_ref: None,
            actor_ref: SYSTEM,
            published_at: at,
        },
    )
    .await
    .expect("freeze a version");
}

/// Freeze one entity version whose stored digest does **not** match its
/// content — a restored row that rotted.
async fn seed_corrupt_entity_version(
    h: &Harness,
    entity_id: Uuid,
    version: i64,
    at: chrono::DateTime<Utc>,
    digest_version: i32,
) {
    let conn = h.db.conn().expect("connection");
    repo::insert_entity_version(
        &conn,
        &scope(),
        repo::NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: repo::VersionedEntityKind::Product,
            entity_id,
            published_version: version,
            content: format!("{{\"name\":\"v{version}\"}}"),
            content_digest: vec![0x00],
            digest_version,
            approval_ref: None,
            actor_ref: SYSTEM,
            published_at: at,
        },
    )
    .await
    .expect("freeze a rotted version");
}

/// Commit one catalog version whose manifest names `entries`.
async fn seed_catalog_version(
    h: &Harness,
    catalog_version_id: i64,
    at: chrono::DateTime<Utc>,
    entries: &[(Uuid, i64)],
) {
    seed_catalog_version_with_participants(h, catalog_version_id, at, entries, "[]").await;
}

/// The same, with an explicit participant snapshot — a **non-empty** one has
/// members who owe an ack, so `domain::retention::evaluate` holds the version
/// until the ledger says otherwise.
async fn seed_catalog_version_with_participants(
    h: &Harness,
    catalog_version_id: i64,
    at: chrono::DateTime<Utc>,
    entries: &[(Uuid, i64)],
    participants: &str,
) {
    let conn = h.db.conn().expect("connection");
    let refs: Vec<repo::SnapshotEntityRef> = entries
        .iter()
        .map(|(entity_id, version)| repo::SnapshotEntityRef {
            entity_kind: "product".to_owned(),
            entity_id: *entity_id,
            published_version: *version,
            lifecycle_state: "published".to_owned(),
        })
        .collect();
    let manifest = crate::infra::increment::VersionManifest {
        entries: refs.clone(),
        captures: Vec::new(),
        participant_set: Vec::new(),
    };
    repo::insert_catalog_version(
        &conn,
        &scope(),
        TENANT,
        repo::NewCatalogVersion {
            catalog_version_id,
            checksum: manifest.checksum(),
            digest_version: canonical::DIGEST_VERSION,
            published_at: at,
            // An EMPTY participant set reads as collectable — nobody ever
            // owed an ack — so a case seeding `[]` measures the storage arm
            // and one seeding a member measures the freeze gate.
            participant_set_snapshot: participants.to_owned(),
            freeze_state: crate::domain::states::FreezeState::Complete,
        },
    )
    .await
    .expect("commit a catalog version");
    if !refs.is_empty() {
        repo::insert_catalog_version_entries(&conn, &scope(), TENANT, catalog_version_id, &refs)
            .await
            .expect("write the manifest entries");
    }
}

/// Run one statement and answer the engine's refusal message.
///
/// `test_support::raw_string_opt` **panics** on a driver error, so it cannot
/// assert that a guard fired — it can only assert that one did not. Every
/// case below that names a trigger by its own message needs the error back.
async fn raw_exec_err(dsn: &str, sql: &str) -> String {
    use sea_orm::ConnectionTrait as _;
    let conn = sea_orm::Database::connect(dsn)
        .await
        .expect("open an auxiliary connection for test introspection");
    let refusal = conn
        .execute_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect_err("this statement must be refused")
        .to_string();
    conn.close().await.ok();
    refusal
}

/// One capture row on a version, so the "whole" assertion has all three
/// halves of the chain to lose.
async fn seed_capture(h: &Harness, catalog_version_id: i64, kind: &str) {
    let conn = h.db.conn().expect("connection");
    repo::insert_catalog_version_capture(&conn, &scope(), TENANT, catalog_version_id, kind, "{}")
        .await
        .expect("write a capture");
}

/// One audit row, so the tenant exists for discovery and the audit class has
/// a candidate.
async fn seed_audit_row(h: &Harness, at: chrono::DateTime<Utc>) -> Uuid {
    let conn = h.db.conn().expect("connection");
    let audit_id = Uuid::now_v7();
    repo::write_eventless_act_audit(
        &conn,
        &scope(),
        repo::AuditCommon {
            audit_id,
            tenant_id: TENANT,
            actor_ref: SYSTEM,
            action: "seed".to_owned(),
            subject_kind: "seed".to_owned(),
            reason: None,
            correlation_id: None,
            written_at: at,
        },
        TENANT,
        None,
    )
    .await
    .expect("seed an audit row");
    audit_id
}

/// The `reason` strings of every sweep audit row, newest last.
async fn sweep_audit_reasons(h: &Harness) -> Vec<String> {
    crate::test_support::raw_string_opt(
        &h.dsn,
        "SELECT COALESCE(GROUP_CONCAT(reason, '||'), '') AS v FROM products_audit_log \
         WHERE action = 'retention.sweep' ORDER BY written_at",
    )
    .await
    .map(|joined| {
        if joined.is_empty() {
            Vec::new()
        } else {
            joined.split("||").map(str::to_owned).collect()
        }
    })
    .unwrap_or_default()
}

// -- `dod-retention-clock` --

/// **Each class produces candidates at its own configured window.**
///
/// §6's criterion, and the failure it names: *"a sweep that read one number
/// for every class would pass a single-class probe"*. So one window is moved
/// and the other two classes' candidate counts are asserted **unchanged** —
/// which is the half a single-class probe cannot see.
#[tokio::test]
async fn each_class_reads_its_own_window() {
    let h = harness().await;
    let entity_id = Uuid::from_u128(0xfa_01);
    seed_audit_row(&h, days_ago(100)).await;
    seed_entity_version(&h, entity_id, 1, days_ago(100)).await;
    seed_catalog_version(&h, 1, days_ago(100), &[]).await;

    // Every window at 200 days: nothing is a candidate anywhere.
    let wide = caps(200);
    for class in RecordClass::ALL {
        let outcome = sweep_class(&h.db, &wide, TENANT, class, SYSTEM, Utc::now())
            .await
            .expect("the pass runs");
        assert_eq!(
            outcome.candidates,
            0,
            "{} has a candidate at a 200-day window over 100-day-old rows",
            class.as_str()
        );
    }

    // Narrow ONLY the version window.
    let narrowed = RetentionCaps {
        version_days: 50,
        ..caps(200)
    };
    let version = sweep_class(
        &h.db,
        &narrowed,
        TENANT,
        RecordClass::Version,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert_eq!(
        version.candidates, 1,
        "the version class must see its own narrowed window"
    );
    for other in [RecordClass::Financial, RecordClass::Audit] {
        let outcome = sweep_class(&h.db, &narrowed, TENANT, other, SYSTEM, Utc::now())
            .await
            .expect("the pass runs");
        assert_eq!(
            outcome.candidates,
            0,
            "{} moved with a window that is not its own",
            other.as_str()
        );
    }
}

/// **The two deliberately-excluded populations produce no candidates.**
///
/// Asserted as absence — an over-broad clock deletes records nobody asked it
/// to — and asserted by **name**, so a sweep that grew a fourth class over
/// one of these tables is a red rather than a silent widening.
#[tokio::test]
async fn the_excluded_populations_are_never_candidates() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(100)).await;

    let names = [
        "outbox",
        "products_reference_watermark",
        "products_reference_member",
    ];
    let source = include_str!("storage/repo/retention_gc.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    let code: String = production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for name in names {
        assert!(
            !code.contains(name),
            "{name} is outside every clock (P-D-22 for the outbox, operational current state \
             for 07's tables) and the sweep's store layer must not reach it"
        );
    }

    // And the live half: with every window at zero, so that EVERYTHING the
    // sweep knows about is a candidate, the three classes still see only
    // what they own.
    let all = caps(0);
    let mut total = 0_u32;
    for class in RecordClass::ALL {
        total += sweep_class(&h.db, &all, TENANT, class, SYSTEM, Utc::now())
            .await
            .expect("the pass runs")
            .candidates;
    }
    assert!(
        total > 0,
        "the zero-window control must make something a candidate, or the absence above is \
         proven by a sweep that sees nothing at all"
    );
}

/// **A class whose table refuses `DELETE` is reported held, and the sweep's
/// other candidates still complete.**
///
/// §6's criterion, and the `P0001` failure it guards: one flat refusal is not
/// retryable contention, so a sweep that judged the class in one transaction
/// would abort and take every unrelated candidate with it. Both halves here —
/// the audit class holds, and the version class in the same tenant collects.
#[tokio::test]
async fn a_refusing_class_is_held_and_the_others_still_collect() {
    let h = harness().await;
    let entity_id = Uuid::from_u128(0xfa_02);
    seed_audit_row(&h, days_ago(100)).await;
    // An entity version NO manifest references: `m20260829_000007`'s
    // referential predicate admits exactly this.
    seed_entity_version(&h, entity_id, 1, days_ago(100)).await;

    let narrow = caps(1);
    let audit = sweep_class(
        &h.db,
        &narrow,
        TENANT,
        RecordClass::Audit,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert!(audit.candidates >= 1, "the seeded audit row is a candidate");
    assert_eq!(audit.collected, 0, "the audit plane admits no DELETE");
    assert_eq!(audit.held, audit.candidates, "every candidate is held");
    assert_eq!(
        audit.held_reason,
        Some("retention_storage_refused"),
        "the hold names the storage guard, not a generic skip"
    );

    let version = sweep_class(
        &h.db,
        &narrow,
        TENANT,
        RecordClass::Version,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert_eq!(
        (version.candidates, version.collected, version.held),
        (1, 1, 0),
        "the unreferenced version collects even though the audit class refused in the same \
         sweep - which is the whole of what per-candidate transactions buy"
    );
}

/// **Every pass is audited with its class, its clock and its verdict.**
///
/// Read off the row rather than off the return value: a typed outcome proves
/// the sweep computed something, and the `DoD` obliges it to have **recorded**
/// it.
#[tokio::test]
async fn every_pass_writes_an_audit_row_carrying_its_class_clock_and_verdict() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(100)).await;

    for class in RecordClass::ALL {
        sweep_class(&h.db, &caps(1), TENANT, class, SYSTEM, Utc::now())
            .await
            .expect("the pass runs");
    }
    let reasons = sweep_audit_reasons(&h).await;
    assert_eq!(reasons.len(), 3, "one row per class per pass");
    for class in RecordClass::ALL {
        let row = reasons
            .iter()
            .find(|r| r.contains(&format!("class={}", class.as_str())))
            .unwrap_or_else(|| panic!("no audit row for {}: {reasons:?}", class.as_str()));
        assert!(row.contains("cutoff="), "the clock is on the row: {row}");
        assert!(
            row.contains("held_reason="),
            "the verdict is on the row: {row}"
        );
    }
    let audit_row = reasons
        .iter()
        .find(|r| r.contains("class=audit"))
        .expect("the audit class has a row");
    assert!(
        audit_row.contains("held_reason=retention_storage_refused"),
        "the held verdict is a recorded row and not an inference: {audit_row}"
    );
}

// -- `dod-retention-order` --

/// **An entity-version row referenced by a retained catalog version survives
/// its own class clock — and it is the GUARD that refuses, not the GC.**
///
/// §6's two criteria in one case, because they are two halves of one claim
/// and the second is the one that matters: *"refused **by the guard**, not
/// merely skipped by the GC — the probe passes even when the GC is bypassed
/// entirely"*. So the second half issues the `DELETE` **directly**, with no
/// sweep involved at all.
#[tokio::test]
async fn a_referenced_version_is_refused_by_the_guard_and_not_by_the_sweep() {
    let h = harness().await;
    let referenced = Uuid::from_u128(0xfa_03);
    let orphan = Uuid::from_u128(0xfa_04);
    seed_audit_row(&h, days_ago(100)).await;
    seed_entity_version(&h, referenced, 1, days_ago(100)).await;
    seed_entity_version(&h, orphan, 1, days_ago(100)).await;
    seed_catalog_version(&h, 1, days_ago(100), &[(referenced, 1)]).await;

    let outcome = sweep_class(
        &h.db,
        &caps(1),
        TENANT,
        RecordClass::Version,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert_eq!(
        (outcome.candidates, outcome.collected, outcome.held),
        (2, 1, 1),
        "the orphan collects and the referenced row is held"
    );
    assert_eq!(
        outcome.held_reason,
        Some("retention_manifest_referenced"),
        "the hold names the derive rule"
    );

    // The half that matters: the guard refuses the same DELETE with the GC
    // bypassed entirely.
    let conn = h.db.conn().expect("connection");
    let direct = repo::delete_entity_version(
        &conn,
        &scope(),
        TENANT,
        &repo::EntityVersionKey {
            entity_kind: "product".to_owned(),
            entity_id: referenced,
            published_version: 1,
        },
    )
    .await;
    assert!(
        direct.is_err(),
        "P-D-40's referential predicate must refuse this with no GC in the picture; a green \
         sweep over a table that admits everything would prove only that the sweep skipped"
    );
}

/// **A released catalog version collects whole** — the manifest row, its
/// entries and its captures, in one transaction (**P-D-137**, P-D-118 item
/// 25).
///
/// Before P-D-137 this case asserted the opposite, and correctly: the chain
/// refused every `DELETE`, so *"one catalog version at a time, whole"*
/// described a transaction that always rolled back. The arms are open now and
/// the assertion is the other half of the same claim.
#[tokio::test]
async fn a_released_catalog_version_collects_whole() {
    let h = harness().await;
    let entity_id = Uuid::from_u128(0xfa_05);
    seed_audit_row(&h, days_ago(100)).await;
    seed_entity_version(&h, entity_id, 1, days_ago(100)).await;
    seed_catalog_version(&h, 1, days_ago(100), &[(entity_id, 1)]).await;
    seed_capture(&h, 1, "roster").await;

    let outcome = sweep_class(
        &h.db,
        &caps(1),
        TENANT,
        RecordClass::Financial,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert_eq!(
        (outcome.candidates, outcome.collected, outcome.held),
        (1, 1, 0),
        "the release stamp admits the whole chain"
    );

    let conn = h.db.conn().expect("connection");
    assert!(
        repo::find_catalog_version(&conn, &scope(), TENANT, 1)
            .await
            .expect("read the version back")
            .is_none(),
        "the manifest row is gone"
    );
    let (entries, captures) = repo::catalog_version_manifest_rows(&conn, &scope(), TENANT, 1)
        .await
        .expect("read the manifest back");
    assert!(
        entries.is_empty() && captures.is_empty(),
        "and so are its entries and captures - a surviving entry beside a deleted manifest is \
         the orphan the FK forbids and item 25's boundary exists to prevent"
    );
}

/// **A version the freeze gate holds keeps its entries — and the hold is the
/// gate's, not the storage's.**
///
/// The distinction is the whole of C4: a snapshot member with no registration
/// row **holds** the version, and the sweep must not have started deleting on
/// the way to finding out. The reason token separates the two causes.
#[tokio::test]
async fn a_freeze_held_catalog_version_keeps_its_entries() {
    let h = harness().await;
    let entity_id = Uuid::from_u128(0xfa_06);
    seed_audit_row(&h, days_ago(100)).await;
    seed_entity_version(&h, entity_id, 1, days_ago(100)).await;
    seed_catalog_version_with_participants(
        &h,
        1,
        days_ago(100),
        &[(entity_id, 1)],
        "[\"pricing\"]",
    )
    .await;

    let outcome = sweep_class(
        &h.db,
        &caps(1),
        TENANT,
        RecordClass::Financial,
        SYSTEM,
        Utc::now(),
    )
    .await
    .expect("the pass runs");
    assert_eq!(
        (outcome.candidates, outcome.collected, outcome.held),
        (1, 0, 1),
        "a snapshot member with no registration row holds the version"
    );
    assert_eq!(
        outcome.held_reason,
        Some(crate::domain::retention::RetentionHold::REASON),
        "the hold is the GATE's alarm, not the storage guard's - an operator filtering on \
         retention_orphan_blocked must find it"
    );

    let conn = h.db.conn().expect("connection");
    let (entries, _) = repo::catalog_version_manifest_rows(&conn, &scope(), TENANT, 1)
        .await
        .expect("read the manifest back");
    assert_eq!(
        entries.len(),
        1,
        "nothing was deleted on the way to the hold"
    );
    assert!(
        repo::find_catalog_version(&conn, &scope(), TENANT, 1)
            .await
            .expect("read the version back")
            .is_some(),
        "the version row stands"
    );
    // Read the stamp with a COUNT rather than off `CatalogVersionRecord`:
    // that struct is `06`'s and carries what the resolver needs, and a
    // column no production reader wants does not belong on it. `COUNT`
    // rather than the value, because `raw_string_opt` panics on no row and
    // this assertion needs zero to be an answer.
    assert_eq!(
        crate::test_support::raw_i64(
            &h.dsn,
            "SELECT COUNT(*) AS v FROM products_catalog_version \
             WHERE retention_released_at IS NOT NULL",
        )
        .await,
        0,
        "and it was never stamped: the stamp is what makes a version deletable, so stamping one \
         the gate holds would leave a released version behind for the next pass to collect \
         without ever re-asking the gate"
    );
}

/// **An unreleased version's `DELETE` is refused by the guard, with the GC
/// bypassed entirely** — and a released one's is admitted.
///
/// The same shape `dod-retention-order`'s entity-version case takes, for the
/// same reason: a green sweep over a table that admits everything proves only
/// that the sweep skipped.
#[tokio::test]
async fn the_release_stamp_is_what_the_delete_arm_reads() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(100)).await;
    seed_catalog_version(&h, 1, days_ago(100), &[]).await;
    let conn = h.db.conn().expect("connection");

    let unreleased = repo::delete_catalog_version(&conn, &scope(), TENANT, 1).await;
    assert!(
        unreleased.is_err(),
        "an unstamped version is refused by m20260901_000010's arm, with no GC in the picture"
    );

    assert!(
        repo::stamp_retention_release(
            &conn,
            &scope(),
            TENANT,
            1,
            canonical::write_instant(Utc::now())
        )
        .await
        .expect("the stamp is admitted once"),
        "the whitelist admits NULL -> a value"
    );
    repo::delete_catalog_version(&conn, &scope(), TENANT, 1)
        .await
        .expect("a stamped version is admitted");
}

/// **The stamp moves once and never again.**
///
/// `NULL` → a value is admitted; a second stamp is not. Without the once-only
/// arm the column is a toggle a caller could flip around a delete, and the
/// "deliberate two-step" the arm buys would be worth nothing.
#[tokio::test]
async fn the_release_stamp_cannot_be_moved_or_cleared() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(100)).await;
    seed_catalog_version(&h, 1, days_ago(100), &[]).await;
    let conn = h.db.conn().expect("connection");
    let now = canonical::write_instant(Utc::now());

    assert!(
        repo::stamp_retention_release(&conn, &scope(), TENANT, 1, now)
            .await
            .expect("the first stamp is admitted")
    );
    // The repo's own `WHERE retention_released_at IS NULL` makes a second
    // call a lost race rather than a driver error - which is the behaviour a
    // racing pass needs. The TRIGGER is what refuses a rewrite, and it is
    // reached by an update that does not carry that predicate.
    assert!(
        !repo::stamp_retention_release(&conn, &scope(), TENANT, 1, now)
            .await
            .expect("a second call is a lost race, not a failure"),
        "the second stamp affects no row"
    );
    let rewrite = raw_exec_err(
        &h.dsn,
        "UPDATE products_catalog_version SET retention_released_at = '2030-01-01T00:00:00Z'",
    )
    .await;
    assert!(
        rewrite.contains("stamped once and never moved"),
        "the trigger refuses a rewrite that bypasses the repo's predicate, by name: {rewrite}"
    );
    let clear = raw_exec_err(
        &h.dsn,
        "UPDATE products_catalog_version SET retention_released_at = NULL",
    )
    .await;
    assert!(
        clear.contains("stamped once and never moved"),
        "and refuses a clear - a stamp that could be withdrawn is a toggle, not a release: \
         {clear}"
    );
}

// -- `dod-erasure-age` --

/// Mint a live map entry whose `last_seen_at` is `days` old.
async fn seed_aged_principal(h: &Harness, principal_ref: &str, days: i64) -> Uuid {
    let conn = h.db.conn().expect("connection");
    repo::resolve_actor_ref(&conn, &scope(), TENANT, principal_ref, days_ago(days))
        .await
        .expect("mint the principal's ref")
}

/// Read one map entry back through the entity.
async fn map_entry(h: &Harness, principal_ref: &str) -> repo::IdentityEntry {
    let conn = h.db.conn().expect("connection");
    repo::identity_entries_of_principal(&conn, &scope(), TENANT, principal_ref)
        .await
        .expect("read the map")
        .into_iter()
        .next()
        .expect("the principal has an entry")
}

/// **The age trigger fires without a request, and leaves a fresh principal
/// alone.**
///
/// §6's criterion, and the criterion the brief asked for red first: *"an aged
/// principal and a fresh one, one tombstone"*. The negative half is the one
/// that matters — a sweep that tombstoned everything would satisfy the
/// positive half perfectly.
#[tokio::test]
async fn the_age_trigger_tombstones_the_aged_principal_and_only_that_one() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(1)).await;
    seed_aged_principal(&h, "principal:old", 900).await;
    seed_aged_principal(&h, "principal:new", 10).await;

    let sink = crate::infra::broker::EventSink::Interim(Arc::clone(&h.outbox));
    super::tombstone_aged_principals(
        &h.db,
        &sink,
        &caps(730),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let old = map_entry(&h, "principal:old").await;
    assert!(
        old.tombstoned_at.is_some(),
        "the principal two years past its last activity is tombstoned"
    );
    assert!(old.identity_payload.is_none(), "the payload is destroyed");

    let new = map_entry(&h, "principal:new").await;
    assert!(
        new.tombstoned_at.is_none(),
        "the fresh principal is untouched: a sweep that tombstoned both would pass the positive \
         half and be the M2 failure the operand was chosen to prevent"
    );
}

/// **The age path is the requested path's own act: one code path, one
/// event.**
///
/// The map state is asserted identical to what the door leaves, and the same
/// `ActorErased` is announced. What differs by construction is the audit
/// row's actor and reason (P-D-117 item 14), and that is asserted too — a
/// row indistinguishable from the door's would hide which path ran.
#[tokio::test]
async fn the_age_path_writes_the_same_map_state_and_announces_the_same_event() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(1)).await;
    seed_aged_principal(&h, "principal:old", 900).await;

    let sink = crate::infra::broker::EventSink::Interim(Arc::clone(&h.outbox));
    super::tombstone_aged_principals(
        &h.db,
        &sink,
        &caps(730),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let entry = map_entry(&h, "principal:old").await;
    // Byte-identical in effect IS the map state (P-D-117 item 14): payload
    // destroyed, tombstone stamped, `principal_ref` standing.
    assert!(entry.identity_payload.is_none() && entry.tombstoned_at.is_some());

    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let announced = crate::test_support::raw_i64(
        &h.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE payload_type = 'ActorErased'"),
    )
    .await;
    assert_eq!(announced, 1, "the same event the door emits, once");

    let reason = crate::test_support::raw_string_opt(
        &h.dsn,
        "SELECT reason AS v FROM products_audit_log WHERE action = 'erasure.execute'",
    )
    .await
    .expect("the act wrote its evidential row");
    assert!(
        reason.contains("inst-er-age"),
        "the age path's reason names the age rule, because no human supplied one: {reason}"
    );

    let actor = crate::test_support::raw_i64(
        &h.dsn,
        "SELECT COUNT(*) AS v FROM products_audit_log \
         WHERE action = 'erasure.execute' AND reason IS NOT NULL",
    )
    .await;
    assert_eq!(actor, 1, "one act, one evidential row");
}

/// **A second pass over an already-tombstoned principal does nothing.**
///
/// `tombstoned_at` is *"set once, by erasure, and never cleared"*, and the
/// candidate read excludes tombstoned rows — so the cadence cannot restamp a
/// column the entity's own doc pins, and cannot announce a second erasure of
/// a principal already erased.
#[tokio::test]
async fn a_second_age_pass_neither_restamps_nor_re_announces() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(1)).await;
    seed_aged_principal(&h, "principal:old", 900).await;
    let sink = crate::infra::broker::EventSink::Interim(Arc::clone(&h.outbox));
    let cancel = tokio_util::sync::CancellationToken::new();

    super::tombstone_aged_principals(
        &h.db,
        &sink,
        &caps(730),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &cancel,
    )
    .await;
    let first = map_entry(&h, "principal:old").await.tombstoned_at;
    super::tombstone_aged_principals(
        &h.db,
        &sink,
        &caps(730),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &cancel,
    )
    .await;
    let second = map_entry(&h, "principal:old").await.tombstoned_at;

    assert_eq!(
        first, second,
        "the tombstone instant is set once and never moved"
    );
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    assert_eq!(
        crate::test_support::raw_i64(
            &h.dsn,
            &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE payload_type = 'ActorErased'")
        )
        .await,
        1,
        "one erasure, one announcement, however many times the cadence fires"
    );
}

// -- `dod-restore-drill` --

/// A second database, seeded as the restored copy the platform would provide.
async fn restored_copy() -> Harness {
    harness().await
}

/// Point the drill at `target`.
fn caps_with_target(target: &Harness) -> RetentionCaps {
    RetentionCaps {
        drill_target_dsn: Some(target.dsn.clone()),
        ..caps(3650)
    }
}

/// The `reason` of the newest drill audit row — the *"last-verified
/// watermark"*, which P-D-134 item 6 makes a query rather than a table.
async fn drill_watermark(h: &Harness) -> String {
    crate::test_support::raw_string_opt(
        &h.dsn,
        "SELECT reason AS v FROM products_audit_log WHERE action = 'retention.restore_drill' \
         ORDER BY written_at DESC LIMIT 1",
    )
    .await
    .expect("the run wrote its row")
}

/// **A clean restore verifies, and the run is recorded.**
///
/// The control every case below needs: without it, a drill that reported
/// corruption on everything would pass the corruption case and prove nothing.
#[tokio::test]
async fn a_clean_restore_verifies_both_halves() {
    let h = harness().await;
    let target = restored_copy().await;
    seed_audit_row(&h, days_ago(1)).await;
    let entity_id = Uuid::from_u128(0xfb_01);
    seed_entity_version(&target, entity_id, 1, days_ago(10)).await;
    seed_catalog_version(&target, 1, days_ago(10), &[(entity_id, 1)]).await;

    super::run_restore_drill(
        &h.db,
        &caps_with_target(&target),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let row = drill_watermark(&h).await;
    assert!(row.contains("status=ok"), "{row}");
    assert!(row.contains("corrupt=0"), "{row}");
    assert!(row.contains("unverifiable=0"), "{row}");
    // Both halves: the manifest checksum AND the referenced row's digest.
    assert!(
        row.contains("verified=2"),
        "one manifest and one referenced entity version, each verified: {row}"
    );
}

/// **A deliberately corrupted sample fails the drill loudly.**
///
/// §6's criterion: *"the oracle must be seen to fail"*. The corruption is a
/// stored digest that does not match its own content — which is exactly what
/// a bit-rotted backup looks like from the drill's side.
#[tokio::test]
async fn a_corrupted_restore_raises_the_alarm() {
    let h = harness().await;
    let target = restored_copy().await;
    seed_audit_row(&h, days_ago(1)).await;
    let entity_id = Uuid::from_u128(0xfb_02);
    // Seeded corrupt rather than corrupted afterwards: the frozen guard
    // refuses every `UPDATE` on `products_entity_version`, which is the
    // property the drill exists to be the backstop for — a restore's bytes
    // can rot where a live row's cannot.
    seed_corrupt_entity_version(
        &target,
        entity_id,
        1,
        days_ago(10),
        canonical::DIGEST_VERSION,
    )
    .await;
    seed_catalog_version(&target, 1, days_ago(10), &[(entity_id, 1)]).await;

    super::run_restore_drill(
        &h.db,
        &caps_with_target(&target),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let row = drill_watermark(&h).await;
    assert!(
        row.contains("corrupt=1"),
        "the oracle must be SEEN to fail, and the count is what an operator reads: {row}"
    );
    assert!(
        row.contains("unverifiable=0"),
        "a real mismatch is an alarm, never the version-mismatch warning: {row}"
    );
}

/// **A row written under an earlier `digest_version` is `unverifiable`, and
/// is distinguished from corruption in the result.**
///
/// §6's criterion says *"in the result, not only in a log line"*, which is why
/// the assertion reads the audit row's counts rather than a captured log.
/// P-D-133 item 7: report, never skip, never re-render.
#[tokio::test]
async fn a_foreign_digest_version_is_unverifiable_and_not_corruption() {
    let h = harness().await;
    let target = restored_copy().await;
    seed_audit_row(&h, days_ago(1)).await;
    let entity_id = Uuid::from_u128(0xfb_03);
    // A digest this build has no recomputation code for, and a digest that
    // does not match either — so a drill that re-rendered instead of
    // reporting would raise the corruption alarm here, which is exactly the
    // confusion P-D-133 item 7 forbids.
    seed_corrupt_entity_version(&target, entity_id, 1, days_ago(10), 99).await;
    seed_catalog_version(&target, 1, days_ago(10), &[(entity_id, 1)]).await;

    super::run_restore_drill(
        &h.db,
        &caps_with_target(&target),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let row = drill_watermark(&h).await;
    assert!(
        row.contains("unverifiable=1"),
        "a row this build cannot recompute counts unverifiable: {row}"
    );
    assert!(
        row.contains("corrupt=0"),
        "and is NOT a corruption alarm - re-rendering it under today's rule would manufacture \
         the mismatch the DoD forbids: {row}"
    );
}

/// **With no target configured the run still writes its row, outcome
/// `no_target`.**
///
/// **P-D-135**: a drill that cannot run is not a passed drill, and silence is
/// what *"report, never skip"* forbids.
#[tokio::test]
async fn an_unconfigured_drill_still_records_its_run() {
    let h = harness().await;
    seed_audit_row(&h, days_ago(1)).await;

    super::run_restore_drill(
        &h.db,
        &caps(3650),
        SYSTEM,
        canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let row = drill_watermark(&h).await;
    assert!(
        row.contains("status=no_target"),
        "the run is recorded with the outcome that names why it verified nothing: {row}"
    );
    assert!(
        row.contains("verified=0") && row.contains("corrupt=0"),
        "and claims nothing it did not check: {row}"
    );
}

/// **The digest the drill recomputes excludes the four moving columns.**
///
/// P-D-24, extended by P-D-35: `lifecycle_state`, `deprecation_provenance`,
/// `replaced_by_sku_id` and `internal_revision` move on transitions that
/// write **no** version row, so a drill expecting them would raise an alarm
/// every time a published entity was deprecated.
///
/// Asserted structurally rather than by transition, because the property is
/// about what the drill **reads**: it recomputes over the stored `content`
/// column and nothing else, so a column outside that string cannot reach the
/// comparison however it moves. A behavioural probe would prove one
/// transition; this proves the shape.
#[test]
fn the_drill_recomputes_over_the_stored_content_and_nothing_else() {
    let source = include_str!("retention.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    let code: String = production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("content_digest(&row.content)"),
        "the recomputation's operand is the stored canonical rendering"
    );
    for moving in [
        "lifecycle_state",
        "deprecation_provenance",
        "replaced_by_sku_id",
        "internal_revision",
    ] {
        assert!(
            !code.contains(moving),
            "{moving} moves on a transition that writes no version row (P-D-24, P-D-35); the \
             drill reading it would alarm on every ordinary deprecation"
        );
    }
}

/// A published Product head at `published_version`.
async fn seed_published_product(h: &Harness, product_id: Uuid, published_version: i64) {
    let conn = h.db.conn().expect("connection");
    repo::insert_product(
        &conn,
        &scope(),
        repo::NewProduct {
            product_id,
            tenant_id: TENANT,
            brand_id: Uuid::from_u128(0xb1),
            name: format!("p{product_id}"),
            name_normalized: format!("p{product_id}"),
            product_code: None,
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: SYSTEM.to_string(),
            created_at: days_ago(400),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("create the head");
    // `published_version` **only moves by +1** — the head guard's own rule —
    // so the row is walked up one publish at a time rather than set. That is
    // the schema teaching the fixture what a real head looks like.
    for step in 1..=published_version {
        set_published_version(h, product_id, step).await;
    }
}

/// **A version row a live head names as its current `published_version` is
/// never a candidate** (**P-D-137** (i)).
///
/// The failure it prevents: the schema's only `DELETE` predicate here is
/// P-D-40's manifest reference, so an entity published once — longer ago than
/// `retention_days_version` — and never captured into a manifest would lose
/// its **only** frozen content, and its head would name a `published_version`
/// that does not exist. Both halves: the live head's version survives, and a
/// superseded predecessor of the same entity does not.
#[tokio::test]
async fn a_live_heads_current_version_is_never_a_candidate() {
    let h = harness().await;
    let product_id = Uuid::from_u128(0xfc_01);
    seed_audit_row(&h, days_ago(100)).await;
    // Two frozen versions of one entity, both past the window.
    seed_entity_version(&h, product_id, 1, days_ago(300)).await;
    seed_entity_version(&h, product_id, 2, days_ago(200)).await;
    seed_published_product(&h, product_id, 2).await;

    let conn = h.db.conn().expect("connection");
    let candidates = repo::entity_version_candidates(&conn, &scope(), TENANT, days_ago(1))
        .await
        .expect("the candidate read runs");
    let versions: Vec<i64> = candidates.iter().map(|k| k.published_version).collect();
    assert_eq!(
        versions,
        vec![1],
        "the superseded predecessor is a candidate and the head's current version is not; a \
         sweep without this exclusion collects the only frozen content a live head names"
    );
}

/// Move a head one publish forward through the raw channel.
///
/// This suite is about the sweep rather than about publishing, so the columns
/// are set directly — but **the head guard still judges the write**, and two
/// of its rules shaped this helper rather than being worked around:
/// `published_version` moves by `+1` and `internal_revision` moves by exactly
/// one on every admitted update. A fixture that could not satisfy them would
/// be building a head no door could ever produce, and the sweep's exclusion
/// would then be tested against a row that cannot exist.
async fn set_published_version(h: &Harness, product_id: Uuid, version: i64) {
    use sea_orm::ConnectionTrait as _;
    let conn = sea_orm::Database::connect(&h.dsn)
        .await
        .expect("open an auxiliary connection");
    conn.execute_raw(sea_orm::Statement::from_string(
        sea_orm::DbBackend::Sqlite,
        format!(
            "UPDATE products_product SET published_version = {version}, \
             internal_revision = internal_revision + 1, updated_at = updated_at, \
             lifecycle_state = 'published' WHERE product_id = X'{}'",
            product_id.simple()
        ),
    ))
    .await
    .expect("publish the head");
    conn.close().await.ok();
}

/// **A failure that is not P-D-40's refusal is `StorageRefused`, never the
/// derive rule** (**P-D-137** (ii)).
///
/// Error class follows provenance. Before the fix `collect_entity_version`
/// mapped **every** failure to `ReferencedByRetainedManifest`, so a
/// connection error was audited as a design hold — a row an operator would
/// read as correctly retained when it was never judged at all.
///
/// The forced failure is the table's absence, which is as far from "a
/// manifest references this row" as a failure gets.
#[tokio::test]
async fn a_failure_that_is_not_the_derive_rule_is_reported_as_a_storage_refusal() {
    let h = harness().await;
    let entity_id = Uuid::from_u128(0xfc_02);
    seed_audit_row(&h, days_ago(100)).await;
    seed_entity_version(&h, entity_id, 1, days_ago(100)).await;

    // The candidate read happens first and succeeds; the delete then meets a
    // table that is gone.
    let conn = h.db.conn().expect("connection");
    let candidates = repo::entity_version_candidates(&conn, &scope(), TENANT, days_ago(1))
        .await
        .expect("the candidate read runs");
    assert_eq!(candidates.len(), 1);
    crate::test_support::drop_table(&h.dsn, "products_entity_version").await;

    let outcome = sweep_class(
        &h.db,
        &caps(1),
        TENANT,
        RecordClass::Version,
        SYSTEM,
        Utc::now(),
    )
    .await;
    // The candidate read itself now fails, which is the pass's own error --
    // so the classification is asserted one level down, where the decision
    // actually lives.
    assert!(outcome.is_err(), "the pass reports a failed discovery read");

    let held = super::classify_entity_version_failure("no such table: products_entity_version");
    assert!(
        matches!(
            held,
            crate::domain::retention::HeldReason::StorageRefused(_)
        ),
        "a missing table is a storage refusal, not the derive rule"
    );
    let derive = super::classify_entity_version_failure(
        "products_entity_version: DELETE is admitted only when no products_catalog_version_entry \
         references the row (P-D-40)",
    );
    assert!(
        matches!(
            derive,
            crate::domain::retention::HeldReason::ReferencedByRetainedManifest
        ),
        "and P-D-40's own refusal still is - its message is the operand"
    );
}
