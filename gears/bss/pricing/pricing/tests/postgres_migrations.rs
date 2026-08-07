//! Postgres-only: the migration chain, executed against the backend it targets
//! and **through the runner production uses**.
//!
//! Until this suite existed the chain had never run on Postgres. Every trigger,
//! CHECK and partial index was verified by reading the statement text beside the
//! executed `SQLite` mirror — which proves the two branches say the same thing
//! and proves nothing about whether either is accepted by the server.
//!
//! # Two things this suite is careful about, both learned the hard way
//!
//! **It runs the chain the way the gear boots.** `DatabaseCapability` hands
//! `Migrator::migrations()` to the toolkit runner
//! ([`run_migrations_for_testing`] is its test entry point), which applies in
//! **name** order and books into an *unqualified* `toolkit_migrations_*` table.
//! `MigratorTrait::up` does neither — it applies in vec order and books into
//! `seaql_migrations` — so a suite built on it exercises an ordering production
//! never uses and cannot see the C1 bug class the sibling ledger found: an
//! unqualified bookkeeping table resolving into whichever schema the
//! connection's `search_path` puts first, so that boot 2 finds an empty history
//! and re-runs every migration into a crash loop.
//!
//! **It pins rosters by name, not by count.** A count is satisfied by *any* set
//! of the right size, so a constraint replaced by `CHECK (1 = 1)` keeps a count
//! green — which is exactly how fourteen `pricing_price` CHECKs once could each
//! have been neutralised with the whole suite green. Every CHECK in this chain is
//! uniquely named, so the roster was free and the count was a false economy.
//!
//! This paragraph said "any **62** objects" until 2026-08-04, and by then the
//! roster held 76. That is the small, characteristic way a count rots and a
//! roster does not: the assertion stayed correct because it names its members,
//! while the sentence explaining the assertion carried a number nobody was
//! obliged to update. The number is gone rather than corrected — the rosters
//! below are the count, and a second copy of it in prose is a second thing to
//! keep true.
//!
//! # What a Postgres suite has to do to be evidence
//!
//! **Prove a constraint by executing the statement it must refuse**, and assert
//! the error names that constraint. A test that writes only valid values catches
//! a constraint that got *narrower* and never one that stopped refusing.
//!
//! **Put the world in the state where the object under test is what answers.** A
//! refusal an earlier guard produces is not evidence about the guard named in the
//! test.
//!
//! **Every guard must be provable by removal**: delete the `CONSTRAINT` or the
//! trigger, watch *exactly one* test fail, restore, and report which test it was.
//!
//! **This suite issues no DML and is therefore evidence of presence only.** That
//! the objects reached the server is what it says; that any of them *refuses*
//! what it claims to is Track P's, one executed refusal per object.
//!
//! Ignored by default — they need Docker. Run with
//! `cargo test -p bss-pricing -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::{ConnectOpts, connect_db};

/// Every migration the gear boots with, coord's spliced lease table included.
///
/// **Derived, never written down.** It was a literal `17`, and the literal went
/// stale the day the chain gained `pricing_approval_key`: two boot tests then failed
/// on a count nobody had changed on purpose, which is the same "a count beside a
/// roster, and only one stays true" failure the register migration's own module doc
/// records — here with the roster in code rather than in prose. `Migrator::migrations()`
/// is the roster, so the count comes off it.
///
/// It is more than the `pricing_*` files: the list also carries
/// `coord::migration::Migration::in_schema("bss")`, whose `m0001_…` name sorts
/// **first** under the runner's name ordering and therefore runs before anything
/// in this gear. That is why the number is taken from the list the runner is handed
/// and not from a directory listing.
fn chain_len() -> usize {
    Migrator::migrations().len()
}

/// `testcontainers-modules` defaults to `postgres:11-alpine`, which reached end
/// of life in 2023. Nothing in this repository pins the production server
/// version, so "the backend it targets" is an assumption either way; running on
/// a current major is the closer of the two guesses, and pinning it here means a
/// bump is a diff rather than a dependency's default quietly moving.
const PG_TAG: &str = "16-alpine";

/// A running Postgres, its port, and the container guard.
///
/// The guard is returned because dropping it stops the container: a caller that
/// bound only the port would race its own database to the end of the test.
async fn pg() -> (u16, ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_tag(PG_TAG)
        .start()
        .await
        .expect("start postgres");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("map the postgres port");
    (port, container)
}

/// A DSN carrying `search_path` as a libpq option, the way the gear's runtime
/// config sets it per connection.
fn url_with_search_path(port: u16, search_path: &str) -> String {
    format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres?options=-c%20search_path%3D{search_path}"
    )
}

fn plain_url(port: u16) -> String {
    format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")
}

async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
        .expect("run the catalog query")
        .expect("the catalog query must return a row");
    row.try_get::<i64>("", "n").expect("read the count")
}

/// One `v` column of a catalog query, in the order the query asked for.
async fn names(conn: &DatabaseConnection, sql: &str) -> Vec<String> {
    conn.query_all(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .expect("run the catalog query")
    .iter()
    .map(|row| row.try_get::<String>("", "v").expect("read the name"))
    .collect()
}

const FUNCTIONS_SQL: &str = "SELECT p.proname AS v FROM pg_proc p \
     JOIN pg_namespace n ON n.oid = p.pronamespace \
     WHERE n.nspname = 'bss' ORDER BY 1";
const TRIGGERS_SQL: &str = "SELECT t.tgname AS v FROM pg_trigger t \
     JOIN pg_class c ON c.oid = t.tgrelid \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'bss' AND NOT t.tgisinternal ORDER BY 1";
const CHECKS_SQL: &str = "SELECT co.conname AS v FROM pg_constraint co \
     JOIN pg_namespace n ON n.oid = co.connamespace \
     WHERE n.nspname = 'bss' AND co.contype = 'c' ORDER BY 1";
const PARTIAL_INDEXES_SQL: &str = "SELECT indexname AS v FROM pg_indexes \
     WHERE schemaname = 'bss' AND indexdef LIKE '%WHERE%' ORDER BY 1";
/// Every table's primary key as `table: col, col` (D-236).
///
/// `unnest(conkey) WITH ORDINALITY` is what makes this a mirror of the `SQLite`
/// side rather than a near-miss: `conkey` is the key's **own** column order, and
/// aggregating without it would sort by `attnum` — the order the columns were
/// declared in — so a composite key rearranged into a different key would read
/// identical on both engines.
const PRIMARY_KEYS_SQL: &str = "SELECT c.relname || ': ' \
     || string_agg(a.attname, ', ' ORDER BY k.ord) AS v \
     FROM pg_constraint co \
     JOIN pg_class c ON c.oid = co.conrelid \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     CROSS JOIN LATERAL unnest(co.conkey) WITH ORDINALITY AS k(attnum, ord) \
     JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
     WHERE n.nspname = 'bss' AND co.contype = 'p' \
     GROUP BY c.relname ORDER BY c.relname";

/// The PL/pgSQL functions the chain declares — the objects the `SQLite` mirror
/// cannot carry at all, since it has no procedural language.
///
/// Fewer than the mirror's triggers, and deliberately: `SQLite` needs several
/// literal-message triggers where Postgres needs one function interpolating
/// `TG_OP`. **No count is written here.** It said "eleven" over a roster of
/// thirteen and "forty-seven `CREATE TRIGGER` statements" over a chain that has
/// more than that — a number in prose beside a roster in code is one of them
/// wrong the first time either grows, and the roster is the one the assertion
/// reads.
const EXPECTED_FUNCTIONS: &[&str] = &[
    "pricing_approval_append_only",
    "pricing_approval_key_append_only",
    "pricing_approval_key_follow_state",
    "pricing_approval_threshold_no_delete",
    "pricing_approval_threshold_no_update",
    "pricing_approval_threshold_tombstone_no_delete",
    "pricing_approval_threshold_tombstone_no_update",
    "pricing_audit_log_append_only",
    "pricing_bundle_component_append_only",
    "pricing_bundle_revshare_append_only",
    "pricing_bundle_revshare_group_append_only",
    // Slice 11. One PL/pgSQL function carrying the five arms the SQLite mirror
    // spells as five triggers.
    "pricing_migration_append_only",
    "pricing_plan_addon_rule_append_only",
    "pricing_plan_append_only",
    "pricing_plan_descriptor_set_append_only",
    "pricing_plan_phase_append_only",
    "pricing_price_append_only",
    "pricing_price_overlay_append_only",
    "pricing_price_overlay_line_amount_append_only",
    "pricing_price_overlay_line_append_only",
    "pricing_price_tier_band_append_only",
    "pricing_price_tier_band_kind",
    "pricing_price_tier_band_parent_kind",
    "pricing_price_window_append_only",
    // Slice 11. Two unconditional arms in one function: a migrated-origin
    // snapshot is frozen, so no UPDATE is sanctioned at all.
    "pricing_snapshot_provenance_frozen",
];

/// The triggers those functions are bound to, one per function.
const EXPECTED_TRIGGERS: &[&str] = &[
    "trg_pricing_approval_append_only",
    "trg_pricing_approval_key_append_only",
    "trg_pricing_approval_key_follow_state",
    "trg_pricing_approval_threshold_no_delete",
    "trg_pricing_approval_threshold_no_update",
    "trg_pricing_approval_threshold_tombstone_no_delete",
    "trg_pricing_approval_threshold_tombstone_no_update",
    "trg_pricing_audit_log_append_only",
    "trg_pricing_bundle_component_append_only",
    "trg_pricing_bundle_revshare_append_only",
    "trg_pricing_bundle_revshare_group_append_only",
    "trg_pricing_migration_append_only",
    "trg_pricing_plan_addon_rule_append_only",
    "trg_pricing_plan_append_only",
    "trg_pricing_plan_descriptor_set_append_only",
    "trg_pricing_plan_phase_append_only",
    "trg_pricing_price_append_only",
    "trg_pricing_price_overlay_append_only",
    "trg_pricing_price_overlay_line_amount_append_only",
    "trg_pricing_price_overlay_line_append_only",
    "trg_pricing_price_tier_band_append_only",
    "trg_pricing_price_tier_band_kind",
    "trg_pricing_price_tier_band_parent_kind",
    "trg_pricing_price_window_append_only",
    "trg_pricing_snapshot_provenance_frozen",
];

/// The partial indexes — the `WHERE`-carrying ones, where the predicate *is* the
/// rule (one current revision per plan, one open draft, one terminal phase).
const EXPECTED_PARTIAL_INDEXES: &[&str] = &[
    "idx_pricing_outbox_undrained",
    "idx_pricing_price_supersedes",
    "uq_pricing_approval_key_pending",
    "uq_pricing_approval_policy_pending",
    "uq_pricing_plan_current",
    "uq_pricing_plan_open_draft",
    "uq_pricing_plan_phase_terminal",
    "uq_pricing_price_meter_line_current",
    // D-107. Without the predicate a draft revision of a published overlay
    // collides with itself and an overlay is authorable exactly once.
    "uq_pricing_price_overlay_open_draft",
    "uq_pricing_price_overlay_precedence",
    "uq_pricing_price_scope_key_current",
    "uq_pricing_price_scope_key_draft",
];

/// Every CHECK constraint the chain declares, **by name**.
///
/// Pinned as a roster rather than as a **count**, because a count cannot tell a guard
/// from a tautology: dropping `chk_pricing_plan_revision` and adding `CHECK (1 = 1)` in
/// its place leaves the count exactly where it was while a plan revision of `-999`
/// reaches the table. Every constraint here is uniquely named, so the roster costs
/// nothing the count saved.
///
/// **The number is deleted rather than corrected.** This paragraph said `== 62` twice
/// while the roster below held eighty entries — the file's own opening denounces
/// exactly that shape and had already deleted one number ten lines above. A count
/// beside a roster is one fact with two spellings and only the roster stays true.
/// Seeded from the live server once and hand-checked against the declaring
/// migrations (D-236) — see the `SQLite` roster's note for which four were read
/// back that way and why a roster taken from the code's own output pins the bug.
const EXPECTED_PRIMARY_KEYS: &[&str] = &[
    "coord_leases: key",
    "pricing_approval: approval_id",
    "pricing_approval_key: approval_id, scope_key",
    "pricing_approval_threshold: tenant_id, version, currency",
    "pricing_approval_threshold_tombstone: tenant_id, version",
    "pricing_audit_log: tenant_id, chain_id, seq",
    "pricing_brand_taxonomy: tenant_id, value",
    "pricing_bundle: bundle_id",
    "pricing_bundle_component: bundle_id, plan_revision, component_plan_id",
    "pricing_bundle_revshare: bundle_id, plan_revision, vendor_sku_id, party",
    "pricing_bundle_revshare_group: bundle_id, plan_revision, vendor_sku_id",
    "pricing_catalog_version_ref: tenant_id, pending_ref, subject_kind, subject_ref",
    "pricing_idempotency_dedup: tenant_id, operation, client_key",
    // Read back from `m20260802_000043`'s own DDL rather than from the live
    // server: client-supplied (`inst-ms-api`, M2), so the idempotency key and the
    // primary key are one column.
    "pricing_migration: migration_id",
    "pricing_operator_flag: tenant_id, subject_ref, flag",
    "pricing_org_tier_taxonomy: tenant_id, value",
    "pricing_outbox: outbox_id",
    "pricing_partner_taxonomy: tenant_id, value",
    "pricing_pin_frontier: tenant_id",
    "pricing_plan: plan_id, revision",
    "pricing_plan_addon_rule: plan_id, plan_revision, addon_sku_id",
    "pricing_plan_descriptor_set: plan_id, plan_revision",
    "pricing_plan_phase: phase_id, plan_revision",
    "pricing_policy_object: tenant_id",
    "pricing_price: price_id",
    "pricing_price_overlay: price_overlay_id, revision",
    "pricing_price_overlay_line: line_id, overlay_revision",
    "pricing_price_overlay_line_amount: line_id, overlay_revision, currency",
    "pricing_price_tier_band: band_id",
    "pricing_price_window: window_id",
    "pricing_read_model: tenant_id, catalog_version, subject_kind, subject_ref",
    "pricing_region_taxonomy: tenant_id, value",
    // Read back from `m20260802_000044`'s own DDL rather than from the live server.
    "pricing_snapshot_provenance: provenance_id",
];

const EXPECTED_CHECKS: &[&str] = &[
    "chk_pricing_approval_approver",
    "chk_pricing_approval_decided_at",
    "chk_pricing_approval_distinct_principals",
    "chk_pricing_approval_key_state",
    "chk_pricing_approval_reason",
    "chk_pricing_approval_state",
    "chk_pricing_approval_subject_kind",
    "chk_pricing_approval_threshold_absolute_non_negative",
    "chk_pricing_approval_threshold_basis",
    "chk_pricing_approval_threshold_currency",
    "chk_pricing_approval_threshold_percent_positive",
    "chk_pricing_approval_threshold_tombstone_version",
    "chk_pricing_approval_threshold_version",
    "chk_pricing_audit_log_entry_kind",
    "chk_pricing_audit_log_rollup",
    "chk_pricing_audit_log_seq",
    "chk_pricing_brand_taxonomy_state",
    "chk_pricing_brand_taxonomy_value_present",
    "chk_pricing_bundle_component_min_qty",
    "chk_pricing_bundle_component_qty_range",
    "chk_pricing_bundle_invoice_itemization",
    "chk_pricing_bundle_price_basis",
    "chk_pricing_bundle_revshare_effective_share_bp",
    "chk_pricing_bundle_revshare_group_absorber",
    "chk_pricing_bundle_revshare_group_platform_cut_bp",
    "chk_pricing_bundle_revshare_party",
    "chk_pricing_bundle_revshare_share_bp",
    "chk_pricing_catalog_version_ref_commit",
    "chk_pricing_catalog_version_ref_subject_kind",
    "chk_pricing_catalog_version_ref_subject_lifecycle",
    "chk_pricing_catalog_version_ref_subject_revision",
    "chk_pricing_catalog_version_ref_version",
    "chk_pricing_idempotency_dedup_answered",
    "chk_pricing_idempotency_dedup_status",
    // Slice 11, the same twelve the SQLite mirror carries, name for name.
    "chk_pricing_migration_announced_before_effective",
    "chk_pricing_migration_cancelled_at",
    "chk_pricing_migration_cancelled_order",
    "chk_pricing_migration_completed_at",
    "chk_pricing_migration_completed_order",
    "chk_pricing_migration_distinct_plans",
    "chk_pricing_migration_exclusion_snapshot",
    "chk_pricing_migration_scheduled_unstarted",
    "chk_pricing_migration_source_revision",
    "chk_pricing_migration_started_order",
    "chk_pricing_migration_started_required",
    "chk_pricing_migration_state",
    "chk_pricing_operator_flag_name",
    "chk_pricing_org_tier_taxonomy_state",
    "chk_pricing_org_tier_taxonomy_value_present",
    "chk_pricing_outbox_event_name",
    "chk_pricing_outbox_sequence",
    "chk_pricing_partner_taxonomy_state",
    "chk_pricing_partner_taxonomy_value_present",
    "chk_pricing_pin_frontier_version",
    "chk_pricing_plan_addon_rule_qty_range",
    "chk_pricing_plan_addon_rule_required_max_qty",
    "chk_pricing_plan_addon_rule_step_qty",
    "chk_pricing_plan_availability",
    "chk_pricing_plan_billing_cycle",
    "chk_pricing_plan_custom_interval_n",
    "chk_pricing_plan_custom_interval_pairing",
    "chk_pricing_plan_custom_interval_unit",
    "chk_pricing_plan_frequency",
    "chk_pricing_plan_lifecycle_state",
    "chk_pricing_plan_phase_display_trial_days",
    "chk_pricing_plan_phase_kind",
    "chk_pricing_plan_purchase_qty",
    "chk_pricing_plan_revision",
    "chk_pricing_policy_object_interval_days_cap",
    "chk_pricing_policy_object_interval_months_cap",
    "chk_pricing_policy_object_notice_floor",
    "chk_pricing_policy_object_price_row_cap",
    // Slice 4's C4 switch (`m20260802_000038`), and since D-240
    // (`m20260802_000041`) the only tax-display constraint on this table.
    // `chk_pricing_policy_object_tax_display` stood above it until then, holding
    // a display *basis* default under a name section 6 spends on this
    // fail-closed *enforcement* mode; retiring it is what makes the name
    // unambiguous rather than merely adjacent.
    "chk_pricing_policy_object_tax_display_policy",
    // `chk_pricing_policy_object_threshold` and its `_non_negative` sibling are
    // deliberately absent: `m20260802_000018` dropped the two columns they guarded
    // when the threshold moved to `pricing_approval_threshold`, and a CHECK over a
    // column that no longer exists is what a stale claim looks like.
    "chk_pricing_policy_object_tier_band_cap",
    "chk_pricing_price_aggregation_function",
    "chk_pricing_price_aggregation_granularity",
    "chk_pricing_price_amount_non_negative",
    "chk_pricing_price_billing_granularity",
    "chk_pricing_price_billing_timing",
    "chk_pricing_price_charge_kind",
    "chk_pricing_price_cohort_eligibility",
    "chk_pricing_price_eligibility",
    "chk_pricing_price_grandfather_until",
    "chk_pricing_price_lifecycle_state",
    "chk_pricing_price_manual_quantity",
    "chk_pricing_price_max_hold_granules",
    "chk_pricing_price_model_kind",
    "chk_pricing_price_overlay",
    // Slice 9's seventeen. `chk_pricing_price_overlay` one line up is the
    // **price row's** `price_overlay` axis CHECK (always `base`); everything from
    // here down belongs to the overlay object, which is a separate row.
    "chk_pricing_price_overlay_disclosure",
    "chk_pricing_price_overlay_interval",
    "chk_pricing_price_overlay_lifecycle_state",
    "chk_pricing_price_overlay_line_adjustment_kind",
    "chk_pricing_price_overlay_line_amount_currency",
    "chk_pricing_price_overlay_line_amount_value_minor",
    "chk_pricing_price_overlay_line_cohort_needs_plan",
    "chk_pricing_price_overlay_line_discount_ceiling",
    "chk_pricing_price_overlay_line_fixed_is_amount",
    "chk_pricing_price_overlay_line_magnitude_kind",
    "chk_pricing_price_overlay_line_magnitude_pairing",
    "chk_pricing_price_overlay_line_magnitude_positive",
    "chk_pricing_price_overlay_line_plan_id_not_nil",
    "chk_pricing_price_overlay_line_sku_needs_plan",
    "chk_pricing_price_overlay_line_target_sku_present",
    "chk_pricing_price_overlay_revision",
    "chk_pricing_price_overlay_scope_class",
    "chk_pricing_price_overlay_scope_value",
    "chk_pricing_price_overlay_tax_basis",
    "chk_pricing_price_package_fields_kind",
    "chk_pricing_price_package_price",
    "chk_pricing_price_package_size",
    "chk_pricing_price_quantity_source",
    "chk_pricing_price_tier_aggregation_window",
    "chk_pricing_price_tier_band_from_qty",
    "chk_pricing_price_tier_band_unit_price",
    "chk_pricing_price_tier_band_width",
    "chk_pricing_price_tier_qualification_window",
    "chk_pricing_price_window_activated_at",
    "chk_pricing_price_window_activation_order",
    "chk_pricing_price_window_cancelled_at",
    "chk_pricing_price_window_expired_at",
    "chk_pricing_price_window_expiry_order",
    "chk_pricing_price_window_interval",
    "chk_pricing_price_window_open_ended",
    "chk_pricing_price_window_state",
    "chk_pricing_read_model_catalog_version",
    "chk_pricing_read_model_subject_kind",
    "chk_pricing_read_model_warm_marker",
    "chk_pricing_region_taxonomy_state",
    "chk_pricing_region_taxonomy_value_present",
    "chk_pricing_snapshot_provenance_payload",
    "chk_pricing_snapshot_provenance_resolved",
    "chk_pricing_snapshot_provenance_revision",
    "chk_pricing_snapshot_provenance_trigger",
];

// ---------------------------------------------------------------------------
// The chain through production's runner
// ---------------------------------------------------------------------------

/// Boot 1 applies everything and boot 2 applies nothing.
///
/// This is the C1 regression the sibling ledger pinned, asked of this gear. The
/// hazard is the runner's *unqualified* bookkeeping table: with `bss` first in
/// the path it lands in `public` on boot 1 (before `bss` exists) and a **second,
/// empty** one is created in `bss` on boot 2, whereupon the history reads empty,
/// every migration re-runs, and a non-`IF NOT EXISTS` `CREATE TABLE` aborts the
/// boot. `public` first is the arrangement that cannot do that.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_second_boot_applies_nothing_under_a_public_first_search_path() {
    let (port, _guard) = pg().await;
    let db = connect_db(
        &url_with_search_path(port, "public,bss"),
        ConnectOpts::default(),
    )
    .await
    .expect("connect with a public,bss search_path");

    let first = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 1 must apply the whole chain");
    assert_eq!(first.applied, chain_len(), "boot 1 applies every migration");

    let second = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 2 must be a clean no-op");
    assert_eq!(second.applied, 0, "boot 2 applies nothing");
    assert_eq!(second.skipped, chain_len(), "boot 2 skips every migration");
}

/// **This gear carries C1, and the second boot crash-loops under `bss,public`.**
///
/// Found 2026-08-03 by the first run of this suite, and it is a defect rather
/// than a test artifact: with `bss` first, boot 1 puts the runner's *unqualified*
/// bookkeeping table in `public` because `bss` does not exist yet; boot 2 finds
/// `bss` first, creates a **second, empty** bookkeeping table there, reads an
/// empty history, and re-runs the chain into
/// `relation "coord_leases" already exists`.
///
/// The chain's module doc argues this gear is safe here, and it argues the wrong
/// half: `m20260802_000001` and coord's `m0001_…` do both issue `CREATE SCHEMA IF
/// NOT EXISTS bss`, so *schema creation* is order-proof — but C1 is about where
/// the **bookkeeping** table resolves, which no `IF NOT EXISTS` affects.
///
/// Nothing in this repository configures a `search_path` for this gear, so the
/// hazard is latent rather than live: the server default puts bookkeeping in
/// `public` and the boot above is the one that happens. It becomes live the day a
/// deployment sets `bss,public`, which is the arrangement the sibling ledger
/// shipped and had to fix.
///
/// Pinned as executable documentation in ledger's own idiom
/// (`postgres_migration_idempotency.rs::bss_first_search_path_crash_loops_on_second_boot`).
/// **When the hazard is closed this test reddens, and that is deliberate** — it
/// forces whoever closes it to invert the assertion rather than to discover
/// later that the fix was never exercised.
#[tokio::test]
#[ignore = "requires Docker (testcontainers); documents the C1 crash"]
async fn a_bss_first_search_path_crash_loops_on_the_second_boot() {
    let (port, _guard) = pg().await;
    let db = connect_db(
        &url_with_search_path(port, "bss,public"),
        ConnectOpts::default(),
    )
    .await
    .expect("connect with a bss,public search_path");

    let first = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 1 succeeds even under the hazardous order");
    assert_eq!(first.applied, chain_len());

    let second = run_migrations_for_testing(&db, Migrator::migrations()).await;
    assert!(
        second.is_err(),
        "boot 2 under bss,public must reproduce C1; got {second:?}"
    );

    let raw = Database::connect(&plain_url(port))
        .await
        .expect("connect plainly");
    let bookkeeping = count(
        &raw,
        "SELECT count(*)::bigint AS n FROM information_schema.tables \
         WHERE table_name LIKE 'toolkit_migrations%'",
    )
    .await;
    assert_eq!(
        bookkeeping, 2,
        "and the mechanism is the duplicate bookkeeping table, not something else"
    );
}

// ---------------------------------------------------------------------------
// What reached the server
// ---------------------------------------------------------------------------

/// A chain applied through the runner, for the census tests to inspect.
async fn applied() -> (DatabaseConnection, ContainerAsync<Postgres>) {
    let (port, guard) = pg().await;
    let db = connect_db(
        &url_with_search_path(port, "public,bss"),
        ConnectOpts::default(),
    )
    .await
    .expect("connect");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("apply the chain");
    let raw = Database::connect(&plain_url(port))
        .await
        .expect("connect plainly");
    (raw, guard)
}

/// The PL/pgSQL functions and their triggers, by name.
///
/// What this does **not** say: that any body is correct. `check_function_bodies`
/// is a syntax check — a trigger function may reference a column that does not
/// exist and still be created, failing only when it fires. Whether these
/// triggers refuse what they claim to is Track P's, and needs DML this suite
/// deliberately does not issue.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn every_declared_trigger_function_and_trigger_reaches_the_server() {
    let (conn, _guard) = applied().await;
    assert_eq!(names(&conn, FUNCTIONS_SQL).await, EXPECTED_FUNCTIONS);
    assert_eq!(names(&conn, TRIGGERS_SQL).await, EXPECTED_TRIGGERS);
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn every_table_key_reaches_the_server_with_the_columns_and_the_order_it_was_declared_in() {
    // The Postgres half of D-236's roster. The `SQLite` half is on the fast tier on
    // purpose: D-236's own finding is that the pin which caught `m20260802_000036`
    // was `#[ignore]`d behind Docker, so a run without it reported a clean change —
    // "one premise duplicated across tiers breaks in instalments", arriving from the
    // direction where the premise lived on *one* tier only. This half exists because
    // the two engines declare these keys in separate statements and could drift.
    let (conn, _guard) = applied().await;
    assert_eq!(
        names(&conn, PRIMARY_KEYS_SQL).await,
        EXPECTED_PRIMARY_KEYS,
        "a primary key that lost a column, gained one, or reordered: the one piece of DDL whose \
         loss first shows up as a duplicate row in a table whose whole contract is that it has none"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn every_declared_check_constraint_reaches_the_server_by_name() {
    let (conn, _guard) = applied().await;
    assert_eq!(
        names(&conn, CHECKS_SQL).await,
        EXPECTED_CHECKS,
        "a CHECK that vanished or was renamed is a guard nobody removed on purpose"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn every_declared_partial_index_reaches_the_server() {
    let (conn, _guard) = applied().await;
    assert_eq!(
        names(&conn, PARTIAL_INDEXES_SQL).await,
        EXPECTED_PARTIAL_INDEXES
    );
}

// ---------------------------------------------------------------------------
// Down, and back up
// ---------------------------------------------------------------------------

/// A rolled-back chain leaves no table **and no function**.
///
/// Functions are the class this can actually miss: indexes, triggers and CHECKs
/// go with their table by cascade, while every PL/pgSQL function needs an
/// explicit `DROP FUNCTION` in its migration's `down`. Deleting **any one** of those
/// statements leaves an orphan a tables-only assertion cannot see.
///
/// The number is deleted rather than corrected, for `EXPECTED_CHECKS`' reason: it read
/// "one of those nine statements" over fifteen, and what the test ranges over is the
/// chain's functions as the chain declares them rather than a count of `down` bodies.
///
/// The `bss` schema itself is expected to **survive**: coord and the sibling
/// gears live there, and a `down` that dropped it would take their tables with
/// it.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_chain_rolls_back_leaving_no_table_and_no_function() {
    let (port, _guard) = pg().await;
    let conn = Database::connect(&plain_url(port))
        .await
        .expect("connect postgres");
    Migrator::up(&conn, None).await.expect("apply the chain");
    Migrator::down(&conn, None)
        .await
        .expect("the whole chain must roll back");

    let tables = count(
        &conn,
        "SELECT count(*)::bigint AS n FROM pg_tables WHERE schemaname = 'bss'",
    )
    .await;
    assert_eq!(tables, 0, "a rolled-back chain leaves no table in `bss`");

    let functions = count(
        &conn,
        "SELECT count(*)::bigint AS n FROM pg_proc p \
         JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'bss'",
    )
    .await;
    assert_eq!(
        functions, 0,
        "nor an orphan PL/pgSQL function: each `down` must drop its own"
    );

    let schema = count(
        &conn,
        "SELECT count(*)::bigint AS n FROM information_schema.schemata \
         WHERE schema_name = 'bss'",
    )
    .await;
    assert_eq!(
        schema, 1,
        "and the shared schema survives, because coord and the sibling gears live in it"
    );
}

/// The re-entry that is actually evidence: down, then up again.
///
/// Applying twice in a row proves nothing — the runner filters what it has
/// already booked, so the second call executes no statements at all and the
/// chain's own SQL is not idempotent (`CREATE TABLE` without `IF NOT EXISTS`).
/// Rolling back and re-applying is the run that reaches the statements, and it
/// answers this program's standing question: what does the **second** run of the
/// mechanism read?
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_chain_survives_a_roll_back_and_a_re_apply() {
    let (port, _guard) = pg().await;
    let conn = Database::connect(&plain_url(port))
        .await
        .expect("connect postgres");

    Migrator::up(&conn, None).await.expect("apply");
    let before_functions = names(&conn, FUNCTIONS_SQL).await;
    let before_checks = names(&conn, CHECKS_SQL).await;
    let before_indexes = names(&conn, PARTIAL_INDEXES_SQL).await;

    Migrator::down(&conn, None).await.expect("roll back");
    Migrator::up(&conn, None)
        .await
        .expect("the chain must re-apply onto the ground it cleared");

    assert_eq!(names(&conn, FUNCTIONS_SQL).await, before_functions);
    assert_eq!(names(&conn, CHECKS_SQL).await, before_checks);
    assert_eq!(names(&conn, PARTIAL_INDEXES_SQL).await, before_indexes);
}
