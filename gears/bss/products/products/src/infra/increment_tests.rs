//! Tests for the increment coalescer and its transaction — the window
//! rules (bulk first, the starvation probe `features/catalog-version.md`
//! §7 row 5 obliges), the gapless allocator, the ledger seeding, the
//! byte-identity re-render, and `inst-sn-revalidate`'s stage-vs-commit
//! compare through the [`commit_increment`] seam.

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::EntityTrait as _;
use sea_orm_migration::MigratorTrait;
use serde_json::Value as JsonValue;
use toolkit_db::secure::{AccessScope, SecureEntityExt as _, SecureInsertExt as _};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    BULK_WINDOW_MAX, DrainOutcome, INTERACTIVE_WINDOW, SnapshotBuilder, VersionManifest,
    commit_increment, drain_tenant,
};
use crate::infra::storage::entity::{
    catalog_version, catalog_version_capture, catalog_version_entry, freeze_ack, freeze_participant,
};
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewIncrementRequest, NewProduct};
use bss_products_sdk::increments::IncrementLane;

const TENANT: Uuid = Uuid::from_u128(0x1c_01);
const BRAND: Uuid = Uuid::from_u128(0x1c_02);

struct Harness {
    dsn: String,
    db: DBProvider<DbError>,
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
    let path =
        std::env::temp_dir().join(format!("bss-products-increment-{}.sqlite3", Uuid::new_v4()));
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
        .expect("run this gear's own migrator, coord's lease table included");
    Harness {
        dsn,
        db: DBProvider::<DbError>::new(db),
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// The test clock: a fixed whole-second instant, so window arithmetic is
/// exact and stored instants trivially satisfy P-D-82.
fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap()
}

async fn seed_published_product(harness: &Harness, name: &str) -> Uuid {
    let conn = harness.db.conn().expect("conn");
    let product_id = Uuid::now_v7();
    repo::insert_product(
        &conn,
        &scope(),
        NewProduct {
            product_id,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: name.to_owned(),
            name_normalized: name.to_lowercase(),
            product_code: None,
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: "principal:coalescer-test".to_owned(),
            created_at: t0() - ChronoDuration::hours(1),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("seed the head");
    // Freeze v1 and move the head to published, the way the publish door
    // does — content bytes are not this suite's subject, the reference is.
    repo::insert_entity_version(
        &conn,
        &scope(),
        repo::NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: repo::VersionedEntityKind::Product,
            entity_id: product_id,
            published_version: 1,
            content: format!("{{\"name\":\"{name}\"}}"),
            content_digest: vec![0xab; 32],
            digest_version: 1,
            approval_ref: None,
            actor_ref: Uuid::now_v7(),
            published_at: t0() - ChronoDuration::minutes(30),
        },
    )
    .await
    .expect("freeze v1");
    repo::publish_product_head(
        &conn,
        &scope(),
        TENANT,
        product_id,
        1,
        t0() - ChronoDuration::minutes(30),
    )
    .await
    .expect("move the head to published");
    product_id
}

async fn enqueue(
    harness: &Harness,
    key: &str,
    lane: IncrementLane,
    op: Option<&str>,
    age_secs: i64,
) {
    let conn = harness.db.conn().expect("conn");
    repo::enqueue_increment_request(
        &conn,
        &scope(),
        TENANT,
        NewIncrementRequest {
            source: "pricing",
            request_key: key,
            lane,
            operation_key: op,
            requested_at: t0() - ChronoDuration::seconds(age_secs),
        },
    )
    .await
    .expect("enqueue");
}

/// A closed interactive window commits ONE version: the gapless first id,
/// every entry referencing a published head, the freeze-participant
/// capture, `freeze_state = complete` for the empty participant set, and
/// every satisfied request flipped with its version stamped.
#[tokio::test]
async fn a_closed_interactive_window_commits_one_version() {
    let harness = harness().await;
    let product_a = seed_published_product(&harness, "Alpha Line").await;
    let product_b = seed_published_product(&harness, "Beta Line").await;
    enqueue(&harness, "r-1", IncrementLane::Interactive, None, 6).await;
    enqueue(&harness, "r-2", IncrementLane::Interactive, None, 3).await;

    let outcome = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        outcome,
        DrainOutcome::Committed {
            catalog_version_id: 1,
            satisfied: 2
        },
        "both pending interactive requests land as one version"
    );

    let conn = harness.db.conn().expect("conn");
    let row = repo::find_increment_request(&conn, &scope(), TENANT, "pricing", "r-2")
        .await
        .expect("read")
        .expect("the row exists");
    assert_eq!(row.state, crate::domain::states::RequestState::Coalesced);
    assert_eq!(row.satisfied_by_version_id, Some(1));

    let version = catalog_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .one(&conn)
        .await
        .expect("read the version row")
        .expect("one version row");
    assert_eq!(
        version.freeze_state, "complete",
        "an empty participant set is vacuously complete"
    );
    assert_eq!(version.digest_version, 1);

    let entries = catalog_version_entry::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read entries");
    let mut ids: Vec<Uuid> = entries.iter().map(|e| e.entity_id).collect();
    ids.sort();
    let mut expected = vec![product_a, product_b];
    expected.sort();
    assert_eq!(ids, expected, "one entry per published head");
    assert!(entries.iter().all(|e| e.published_version == 1));

    let captures = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read captures");
    assert_eq!(
        captures.len(),
        3,
        "three capture kinds have shipped sources: the freeze-participant set, the \
         reference-producer set (dod-producer-snapshot) and the metadata maps \
         (dod-metadata-placement); the remaining four arrive as 02's and 03's doors land"
    );
    let kinds: Vec<&str> = captures.iter().map(|c| c.capture_kind.as_str()).collect();
    assert!(kinds.contains(&"freeze_participant_set"));
    assert!(kinds.contains(&"reference_producer_set"));
    assert!(kinds.contains(&"metadata_maps"));
}

/// Demand younger than the window waits; the pass commits nothing.
#[tokio::test]
async fn an_open_window_waits() {
    let harness = harness().await;
    enqueue(&harness, "young", IncrementLane::Interactive, None, 1).await;
    let outcome = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(outcome, DrainOutcome::WindowOpen);
    assert!(INTERACTIVE_WINDOW.as_secs() > 1, "premise of the age above");
}

/// The allocator walks 1, 2 across two drains — gapless by the counter and
/// the shared transaction.
#[tokio::test]
async fn the_allocator_is_gapless_across_drains() {
    let harness = harness().await;
    enqueue(&harness, "g-1", IncrementLane::Interactive, None, 6).await;
    let first = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        first,
        DrainOutcome::Committed {
            catalog_version_id: 1,
            satisfied: 1
        }
    );
    enqueue(&harness, "g-2", IncrementLane::Interactive, None, 6).await;
    let second = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        second,
        DrainOutcome::Committed {
            catalog_version_id: 2,
            satisfied: 1
        }
    );
}

/// The starvation probe (§7 row 5's obliged case): with a bulk window past
/// its hard max AND fresh interactive demand, the bulk batch lands FIRST
/// and whole; the interactive batch takes the next pass. A bulk window
/// still open drains nothing of the bulk batch.
#[tokio::test]
async fn a_steady_interactive_trickle_does_not_defer_a_closed_bulk_window() {
    let harness = harness().await;
    let max = i64::try_from(BULK_WINDOW_MAX.as_secs()).expect("fits");
    enqueue(&harness, "b-1", IncrementLane::Bulk, Some("op-1"), max + 10).await;
    enqueue(&harness, "b-2", IncrementLane::Bulk, Some("op-1"), max - 60).await;
    enqueue(&harness, "i-1", IncrementLane::Interactive, None, 6).await;

    let first = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        first,
        DrainOutcome::Committed {
            catalog_version_id: 1,
            satisfied: 2
        },
        "the closed bulk window lands first, whole, as one version"
    );

    {
        let conn = harness.db.conn().expect("conn");
        let interactive = repo::find_increment_request(&conn, &scope(), TENANT, "pricing", "i-1")
            .await
            .expect("read")
            .expect("exists");
        assert_eq!(
            interactive.state,
            crate::domain::states::RequestState::Pending,
            "the interactive batch was not shredded into the bulk version"
        );
    }

    let second = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        second,
        DrainOutcome::Committed {
            catalog_version_id: 2,
            satisfied: 1
        },
        "the interactive batch takes the next pass"
    );
}

/// An open bulk window alone drains nothing.
#[tokio::test]
async fn an_open_bulk_window_waits_for_its_hard_max() {
    let harness = harness().await;
    enqueue(&harness, "b-only", IncrementLane::Bulk, Some("op-2"), 100).await;
    let outcome = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(outcome, DrainOutcome::WindowOpen);
}

/// A registered participant makes the version `open`, seeds one `pending`
/// ledger row (P-D-67), rides the capture store as a stored copy, and
/// fills the derived cache column.
#[tokio::test]
async fn the_participant_snapshot_seeds_the_ledger() {
    let harness = harness().await;
    {
        let conn = harness.db.conn().expect("conn");
        let model = freeze_participant::ActiveModel {
            tenant_id: Set(TENANT),
            participant: Set("pricing".to_owned()),
            registered_at: Set(t0() - ChronoDuration::hours(2)),
        };
        freeze_participant::Entity::insert(model.clone())
            .secure()
            .scope_with_model(&scope(), &model)
            .expect("scope")
            .exec(&conn)
            .await
            .expect("register the participant");
    }
    enqueue(&harness, "p-1", IncrementLane::Interactive, None, 6).await;

    let outcome = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(
        outcome,
        DrainOutcome::Committed {
            catalog_version_id: 1,
            satisfied: 1
        }
    );

    let conn = harness.db.conn().expect("conn");
    let version = catalog_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .one(&conn)
        .await
        .expect("read")
        .expect("one row");
    assert_eq!(version.freeze_state, "open");
    assert!(version.participant_set_snapshot.contains("pricing"));

    let acks = freeze_ack::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read acks");
    assert_eq!(acks.len(), 1);
    assert_eq!(acks[0].participant, "pricing");
    assert_eq!(acks[0].state, "pending");
}

/// The byte-identity flagship's build-side half: re-rendering the manifest
/// from the STORED rows — entries, captures, and the participant set
/// parsed back out of its own capture — reproduces the stored checksum
/// exactly. No re-collect touches the live heads.
#[tokio::test]
async fn re_rendering_the_stored_manifest_reproduces_the_checksum() {
    let harness = harness().await;
    seed_published_product(&harness, "Gamma Line").await;
    enqueue(&harness, "c-1", IncrementLane::Interactive, None, 6).await;
    drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");

    let conn = harness.db.conn().expect("conn");
    let version = catalog_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .one(&conn)
        .await
        .expect("read")
        .expect("one row");
    let entries = catalog_version_entry::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("entries");
    let captures = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("captures");

    let participant_capture = captures
        .iter()
        .find(|c| c.capture_kind == "freeze_participant_set")
        .expect("the participant capture is stored");
    let participants: Vec<String> = serde_json::from_str::<JsonValue>(&participant_capture.content)
        .expect("the capture is canonical JSON")
        .as_array()
        .expect("an array")
        .iter()
        .map(|v| v.as_str().expect("names").to_owned())
        .collect();

    let rebuilt = VersionManifest {
        entries: entries
            .iter()
            .map(|e| repo::SnapshotEntityRef {
                entity_kind: e.entity_kind.clone(),
                entity_id: e.entity_id,
                published_version: e.published_version,
                // The state is not part of the manifest rendering; it is
                // the revalidation operand only.
                lifecycle_state: String::new(),
            })
            .collect(),
        captures: captures
            .iter()
            .map(|c| (c.capture_kind.clone(), c.content.clone()))
            .collect(),
        participant_set: participants,
    };
    assert_eq!(
        rebuilt.checksum(),
        version.checksum,
        "decode-then-render reproduces the stored bytes; the drill can re-verify years later"
    );
}

/// `inst-sn-revalidate` through the seam: stage, move a head, commit — the
/// pass fails closed as `Restaged`, nothing is written, and the requests
/// stay pending for the next tick's fresh collect.
#[tokio::test]
async fn a_moved_head_between_stage_and_commit_restages_the_pass() {
    let harness = harness().await;
    seed_published_product(&harness, "Delta Line").await;
    enqueue(&harness, "s-1", IncrementLane::Interactive, None, 6).await;

    let staged = {
        let conn = harness.db.conn().expect("conn");
        SnapshotBuilder::collect(&conn, &scope(), TENANT)
            .await
            .expect("stage")
    };

    // The race the AC names: a publish lands between collect and commit.
    seed_published_product(&harness, "Delta Line Second").await;

    let outcome = commit_increment(
        &harness.db,
        TENANT,
        staged,
        vec![("pricing".to_owned(), "s-1".to_owned())],
        t0(),
    )
    .await
    .expect("commit runs");
    assert_eq!(
        outcome,
        DrainOutcome::Restaged,
        "the compare fails the pass closed"
    );

    let conn = harness.db.conn().expect("conn");
    assert!(
        catalog_version::Entity::find()
            .secure()
            .scope_with(&scope())
            .one(&conn)
            .await
            .expect("read")
            .is_none(),
        "nothing was written"
    );
    let row = repo::find_increment_request(&conn, &scope(), TENANT, "pricing", "s-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        row.state,
        crate::domain::states::RequestState::Pending,
        "the request is never lost"
    );
}

/// A held lease skips the pass: single-activeness is the drain worker's,
/// through coord's shared primitive (`dod-coalescer`).
#[tokio::test]
async fn a_held_lease_skips_the_pass() {
    let harness = harness().await;
    enqueue(&harness, "l-1", IncrementLane::Interactive, None, 6).await;

    let lease = coord::LeaseManager::new(harness.db.db());
    let guard = lease
        .acquire(
            &format!("cv-increment:{TENANT}"),
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("the test holds the tenant's lease");

    let outcome = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert_eq!(outcome, DrainOutcome::LeaseHeld);

    guard.release().await.expect("release");
    let retry = drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");
    assert!(
        matches!(retry, DrainOutcome::Committed { .. }),
        "the next pass after release drains normally"
    );
}

/// `freeze_overdue`'s operand (dod-freeze-timeout): an `open` version older
/// than the timeout is named together with its still-pending participants;
/// a settled or young version is not. The timeout fails closed — nothing
/// here flips any state.
#[tokio::test]
async fn the_overdue_scan_names_the_silent_participants() {
    let harness = harness().await;
    {
        let conn = harness.db.conn().expect("conn");
        let model = freeze_participant::ActiveModel {
            tenant_id: Set(TENANT),
            participant: Set("pricing".to_owned()),
            registered_at: Set(t0() - ChronoDuration::hours(3)),
        };
        freeze_participant::Entity::insert(model.clone())
            .secure()
            .scope_with_model(&scope(), &model)
            .expect("scope")
            .exec(&conn)
            .await
            .expect("register");
    }
    enqueue(&harness, "od-1", IncrementLane::Interactive, None, 6).await;
    drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");

    let young = super::overdue_freezes(&harness.db, t0(), 24)
        .await
        .expect("scan");
    assert!(
        young.is_empty(),
        "a version inside the timeout is not named"
    );

    let overdue = super::overdue_freezes(&harness.db, t0() + ChronoDuration::hours(25), 24)
        .await
        .expect("scan");
    assert_eq!(overdue.len(), 1);
    assert_eq!(overdue[0].catalog_version_id, 1);
    assert_eq!(overdue[0].silent_participants, vec!["pricing".to_owned()]);
}

/// `dod-producer-snapshot`: the registered producer set rides the capture
/// store per version, symmetrically with the freeze-participant set, so a
/// historical verdict is judged against the **then-registered** set and
/// onboarding a producer never retro-flips a past decision.
#[tokio::test]
async fn the_registered_producer_set_rides_the_capture_store() {
    let harness = harness().await;
    {
        let conn = harness.db.conn().expect("conn");
        crate::infra::storage::repo::register_reference_producer(
            &conn,
            &scope(),
            TENANT,
            "pricing",
            None,
            t0() - ChronoDuration::hours(1),
        )
        .await
        .expect("register");
    }
    enqueue(&harness, "ps-1", IncrementLane::Interactive, None, 6).await;
    drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");

    let conn = harness.db.conn().expect("conn");
    let captures = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read captures");
    let producers = captures
        .iter()
        .find(|c| c.capture_kind == "reference_producer_set")
        .expect("the producer set is captured beside the participant set");
    assert!(
        producers.content.contains("pricing"),
        "the capture stores the then-registered set: {}",
        producers.content
    );

    // A producer registered AFTER the version must not appear in it.
    crate::infra::storage::repo::register_reference_producer(
        &conn,
        &scope(),
        TENANT,
        "contracts",
        None,
        t0(),
    )
    .await
    .expect("register later");
    let captures = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read captures");
    let producers = captures
        .iter()
        .find(|c| c.capture_kind == "reference_producer_set")
        .expect("the capture");
    assert!(
        !producers.content.contains("contracts"),
        "onboarding never retro-flips a past version's snapshot"
    );
}

/// `dod-metadata-placement`: the map is outside frozen version content, and a
/// `CatalogVersion` captures it **as of its own snapshot instant** — so
/// mutating the map afterwards must leave the old snapshot's checksum
/// unmoved. That byte-identity probe is the `DoD`'s own requirement, and it
/// what distinguishes a captured copy from a reference.
///
/// @cpt-dod:cpt-cf-bss-products-dod-metadata-placement:p2
#[tokio::test]
async fn a_metadata_mutation_after_a_snapshot_does_not_move_its_checksum() {
    let harness = harness().await;
    write_metadata(&harness, "internalOwner", "team-a").await;

    enqueue(&harness, "md-1", IncrementLane::Interactive, None, 6).await;
    drain_tenant(&harness.db, TENANT, t0())
        .await
        .expect("drain");

    let conn = harness.db.conn().expect("conn");
    let first = catalog_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read the versions");
    assert_eq!(first.len(), 1, "one version so far");
    let pinned_checksum = first[0].checksum.clone();
    let rows = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("read the captures");
    let captured = rows
        .iter()
        .find(|c| c.capture_kind == "metadata_maps")
        .expect("the metadata map is captured");
    assert!(
        captured.content.contains("team-a"),
        "the capture holds the map as of the snapshot: {}",
        captured.content
    );
    let pinned_content = captured.content.clone();
    return_pinned(conn);

    // Mutate the live map, then re-read the FROZEN version.
    write_metadata(&harness, "internalOwner", "team-b").await;

    let conn = harness.db.conn().expect("conn");
    let after = catalog_version::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("re-read the versions");
    assert_eq!(
        after[0].checksum, pinned_checksum,
        "a metadata write after the snapshot must not move the frozen checksum"
    );
    let rows = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(&scope())
        .all(&conn)
        .await
        .expect("re-read the captures");
    let captured = rows
        .iter()
        .find(|c| c.capture_kind == "metadata_maps")
        .expect("the capture survives");
    assert_eq!(
        captured.content, pinned_content,
        "the captured copy is a copy: it says team-a after the live map says team-b"
    );
    assert!(
        !captured.content.contains("team-b"),
        "the capture is not a reference to the live row"
    );
}

/// Write one metadata row through raw SQL — the metadata door is
/// `dod-metadata-door`'s and waits on §7 rows 2 and 14, so this seeds the
/// store the capture reads.
/// Return the pinned connection before the next checkout, the harness
/// holding exactly one.
fn return_pinned<T>(conn: T) {
    let _returned = conn;
}

async fn write_metadata(harness: &Harness, key: &str, value: &str) {
    use sea_orm::ConnectionTrait as _;

    let conn = sea_orm::Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection to seed the metadata row");
    let sql = format!(
        "INSERT INTO products_metadata \
         (tenant_id, entity_kind, entity_id, key, value, created_at, updated_at) \
         VALUES (X'{tenant}', 'product', X'{entity}', '{key}', '{value}', \
          '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z') \
         ON CONFLICT (tenant_id, entity_kind, entity_id, key) DO UPDATE SET value = '{value}'",
        tenant = TENANT.simple(),
        entity = uuid::Uuid::from_u128(0x0e_01).simple(),
    );
    conn.execute_unprepared(&sql)
        .await
        .expect("seed the metadata row");
}
