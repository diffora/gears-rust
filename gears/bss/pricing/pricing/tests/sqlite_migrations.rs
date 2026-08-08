//! The migration chain — the Foundation tables and the slice-owned ones that
//! follow them — exercised end to end on an in-memory `SQLite`.
//!
//! Three properties, each a real boot failure mode:
//!
//! 1. **Completeness, and agreement with the entities** — after `up`, every
//!    table can be read through its `SeaORM` entity. That is a stronger check
//!    than "the table exists": `SeaORM` names every column in the `SELECT`, so
//!    a migration and an entity that disagree about a column fail here rather
//!    than at the first production read.
//! 2. **Re-run safety** — a second boot over the same database applies nothing
//!    and skips everything. The sibling ledger carries a whole Postgres
//!    regression for the version of this that bit it (bookkeeping landing in the
//!    wrong schema made every migration re-run and a non-`IF NOT EXISTS`
//!    `CREATE TABLE` abort in a crash loop); the cheap half of that check
//!    belongs in the fast suite.
//! 3. **Reversibility** — `down` then `up` round-trips, so a rollback leaves a
//!    database the chain can walk forward again rather than a half-dropped one.
//!    This one introspects `sqlite_master` directly, because it is also where
//!    the shared `coord_leases` table (spliced in for the singleton warm
//!    re-drive) is checked; `coord` does not export its entity.
//!
//! 4. **The object census** — every trigger and every index the chain creates, by
//!    name, in both directions. That is a fourth property rather than a detail of
//!    the first: a table can exist with its guards missing, and on the backend the
//!    in-crate suite actually runs, a **dropped trigger or index changes nothing
//!    any application-level test can see**. Deleting
//!    `uq_pricing_approval_key_pending` or any of the register's `RAISE(ABORT)`
//!    triggers left the whole suite green — the contention tests are answered by
//!    `approval_repo::find_pending_key_holder`, which is a `SELECT` — so the rule
//!    silently degraded from a constraint to a check. The census is what makes that
//!    a red test.
//!
//! Postgres-backed coverage is testcontainers-gated by convention in this repo
//! and none is added: the append-only guards are mirrored onto `SQLite` as
//! `RAISE(ABORT, ...)` triggers, so `sqlite_append_only.rs` exercises them with
//! no Docker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::infra::storage::entity::{
    approval, approval_key, approval_threshold, approval_threshold_tombstone, audit_log,
    brand_taxonomy, catalog_version_ref, idempotency_dedup, operator_flag, org_tier_taxonomy,
    outbox, partner_taxonomy, pin_frontier, plan, plan_addon_rule, plan_descriptor_set, plan_phase,
    policy_object, price, price_overlay, price_overlay_line, price_overlay_line_amount,
    price_tier_band, price_window, read_model, region_taxonomy,
};
use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::{ConnectionTrait, Database, EntityTrait, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

/// Every table the chain creates, `coord_leases` included.
const EXPECTED_TABLES: &[&str] = &[
    "pricing_plan",
    "pricing_plan_phase",
    "pricing_plan_addon_rule",
    "pricing_plan_descriptor_set",
    "pricing_price",
    "pricing_price_tier_band",
    "pricing_read_model",
    "pricing_catalog_version_ref",
    "pricing_composite_meter",
    "pricing_pin_frontier",
    "pricing_policy_object",
    "pricing_operator_flag",
    "pricing_idempotency_dedup",
    "pricing_outbox",
    "pricing_audit_log",
    "pricing_approval",
    "pricing_approval_key",
    "pricing_approval_threshold",
    "pricing_approval_threshold_tombstone",
    "pricing_price_window",
    "pricing_bundle",
    "pricing_bundle_component",
    "pricing_bundle_revshare_group",
    "pricing_bundle_revshare",
    // Slice 4's four scope-value taxonomies, built on the Slice 9 chain because
    // `inst-plv-scope` validates against them and none of them existed.
    "pricing_region_taxonomy",
    "pricing_brand_taxonomy",
    "pricing_partner_taxonomy",
    "pricing_org_tier_taxonomy",
    // Slice 9's three, in dependency order.
    "pricing_price_overlay",
    "pricing_price_overlay_line",
    "pricing_price_overlay_line_amount",
    // Slice 11's migration plane: one scheduled plan migration and its section 4
    // state machine.
    "pricing_migration",
    // Slice 11's synthesis half: the frozen `migrated-origin` record.
    "pricing_snapshot_provenance",
    "coord_leases",
];

/// Every trigger the chain creates.
///
/// Transcribed rather than counted, and asserted **in both directions**: a trigger
/// missing from the database fails, and a trigger the database has and this list does
/// not fails too. The second half is what makes it a census — a guard added without a
/// line here is a guard nobody decided to add.
const EXPECTED_TRIGGERS: &[&str] = &[
    "trg_pricing_approval_born_submitted",
    "trg_pricing_approval_flip_whitelist",
    "trg_pricing_approval_immutable_once_decided",
    "trg_pricing_approval_key_born_submitted",
    "trg_pricing_approval_key_born_under_a_pending_unit",
    "trg_pricing_approval_key_follow_state",
    "trg_pricing_approval_key_follows_its_unit",
    "trg_pricing_approval_key_follows_once",
    "trg_pricing_approval_key_no_delete",
    "trg_pricing_approval_key_pinned_columns",
    "trg_pricing_approval_no_delete",
    "trg_pricing_approval_pinned_columns",
    "trg_pricing_approval_threshold_no_delete",
    "trg_pricing_approval_threshold_no_update",
    "trg_pricing_approval_threshold_tombstone_no_delete",
    "trg_pricing_approval_threshold_tombstone_no_update",
    "trg_pricing_audit_log_no_delete",
    "trg_pricing_audit_log_no_update",
    "trg_pricing_bundle_component_no_delete",
    "trg_pricing_bundle_component_no_insert",
    "trg_pricing_bundle_component_no_update",
    "trg_pricing_bundle_revshare_group_no_delete",
    "trg_pricing_bundle_revshare_group_no_insert",
    "trg_pricing_bundle_revshare_group_no_update",
    "trg_pricing_bundle_revshare_no_delete",
    "trg_pricing_bundle_revshare_no_insert",
    "trg_pricing_bundle_revshare_no_update",
    // Slice 10's composite meter. Three arms mirroring the one Postgres
    // function, `pricing_plan_phase`'s shape: the parent revision's
    // `lifecycle_state` is the row's, so every verb consults it.
    "trg_pricing_composite_meter_no_delete",
    "trg_pricing_composite_meter_no_insert",
    "trg_pricing_composite_meter_no_update",
    // Slice 11. Five arms, mirroring the one Postgres function: the DELETE ban,
    // the terminal-row ban, the frozen-column whitelist, section 4's edges, and
    // D-65's replay guard on the persisted exclusion set.
    "trg_pricing_migration_exclusion_replay",
    "trg_pricing_migration_flip_whitelist",
    "trg_pricing_migration_frozen_columns",
    "trg_pricing_migration_immutable_history",
    "trg_pricing_migration_no_delete",
    "trg_pricing_plan_addon_rule_no_delete",
    "trg_pricing_plan_addon_rule_no_insert",
    "trg_pricing_plan_addon_rule_no_update",
    "trg_pricing_plan_descriptor_set_no_delete",
    "trg_pricing_plan_descriptor_set_no_insert",
    "trg_pricing_plan_descriptor_set_no_update",
    "trg_pricing_plan_draft_flip_whitelist",
    "trg_pricing_plan_flip_whitelist",
    "trg_pricing_plan_frozen_columns",
    "trg_pricing_plan_no_delete",
    "trg_pricing_plan_phase_no_delete",
    "trg_pricing_plan_phase_no_insert",
    "trg_pricing_plan_phase_no_update",
    "trg_pricing_price_draft_flip_whitelist",
    "trg_pricing_price_flip_whitelist",
    "trg_pricing_price_frozen_columns",
    "trg_pricing_price_grandfather_monotonic",
    "trg_pricing_price_no_delete",
    // Slice 9's ten. The header carries the `pricing_plan` arrangement — a
    // frozen-column whitelist, a draft-exit whitelist, a frozen-state flip
    // whitelist and a no-delete off the draft plane — and the two child tables
    // carry `pricing_bundle_component`'s three-verb guard, which is what freezes
    // a published revision's lines and their money with it (D-92).
    "trg_pricing_price_overlay_draft_exit",
    "trg_pricing_price_overlay_frozen_columns",
    "trg_pricing_price_overlay_frozen_flip",
    "trg_pricing_price_overlay_line_amount_no_delete",
    "trg_pricing_price_overlay_line_amount_no_insert",
    "trg_pricing_price_overlay_line_amount_no_update",
    "trg_pricing_price_overlay_line_no_delete",
    "trg_pricing_price_overlay_line_no_insert",
    "trg_pricing_price_overlay_line_no_update",
    "trg_pricing_price_overlay_no_delete",
    "trg_pricing_price_tier_band_kind_insert",
    "trg_pricing_price_tier_band_kind_update",
    "trg_pricing_price_tier_band_no_delete",
    "trg_pricing_price_tier_band_no_insert",
    "trg_pricing_price_tier_band_no_update",
    "trg_pricing_price_tier_band_parent_kind",
    "trg_pricing_price_window_act_sequence",
    "trg_pricing_price_window_flip_whitelist",
    "trg_pricing_price_window_frozen_columns",
    "trg_pricing_price_window_future_end",
    "trg_pricing_price_window_immutable_history",
    "trg_pricing_price_window_no_delete",
    // Slice 11. Two unconditional arms and no whitelist: a migrated-origin
    // snapshot is **frozen**, so no UPDATE is sanctioned at all.
    "trg_pricing_snapshot_provenance_no_delete",
    "trg_pricing_snapshot_provenance_no_update",
];

/// Every index the chain creates, `uq_` and `idx_` alike.
///
/// The `uq_` half is the load-bearing one: each of those is a **rule** — one current
/// revision per plan, one open draft, one published row per scope key, one pending
/// holder per key — and an index dropped from the chain turns the rule into whatever
/// the application happens to check. Asserted in both directions, for
/// [`EXPECTED_TRIGGERS`]' reason.
const EXPECTED_INDEXES: &[&str] = &[
    "idx_pricing_approval_key_approval",
    "idx_pricing_audit_log_recorded",
    "idx_pricing_audit_log_subject",
    "idx_pricing_bundle_component_plan",
    "idx_pricing_bundle_component_revision",
    "idx_pricing_bundle_revshare_group_revision",
    "idx_pricing_bundle_revshare_revision",
    "idx_pricing_bundle_tenant",
    "idx_pricing_catalog_version_ref_version",
    "idx_pricing_composite_meter_revision",
    "idx_pricing_idempotency_dedup_created",
    "idx_pricing_migration_due",
    "idx_pricing_migration_source",
    "idx_pricing_migration_target",
    "idx_pricing_migration_tenant",
    "idx_pricing_operator_flag_by_flag",
    "idx_pricing_outbox_undrained",
    "idx_pricing_plan_addon_rule_revision",
    "idx_pricing_plan_descriptor_set_revision",
    "idx_pricing_plan_phase_revision",
    "idx_pricing_plan_tenant",
    "idx_pricing_price_overlay_line_amount_tenant",
    "idx_pricing_price_overlay_line_plan",
    "idx_pricing_price_overlay_line_revision",
    "idx_pricing_price_overlay_scope",
    "idx_pricing_price_plan",
    "idx_pricing_price_supersedes",
    "idx_pricing_price_tier_band_price",
    "idx_pricing_price_window_due",
    "idx_pricing_price_window_price",
    "idx_pricing_read_model_resolve",
    "idx_pricing_snapshot_provenance_plan",
    "uq_pricing_approval_key_pending",
    "uq_pricing_approval_policy_pending",
    "uq_pricing_bundle_plan",
    "uq_pricing_composite_meter_output",
    "uq_pricing_outbox_dedup_key",
    "uq_pricing_outbox_sequence",
    "uq_pricing_plan_current",
    "uq_pricing_plan_open_draft",
    "uq_pricing_plan_phase_terminal",
    "uq_pricing_price_meter_line_current",
    // D-42's null-safe line key — an **expression** index over three COALESCEd
    // sentinels, because a plain UNIQUE over three nullable columns admits the
    // very rows section 6 spells as "one default line, one line per plan".
    "uq_pricing_price_overlay_line_key",
    "uq_pricing_price_overlay_open_draft",
    // D-107's partial predicate. Dropping the `WHERE lifecycle_state =
    // 'published'` makes a draft revision of a published overlay collide with
    // itself, so an overlay is authorable exactly once.
    "uq_pricing_price_overlay_precedence",
    "uq_pricing_price_scope_key_current",
    "uq_pricing_price_scope_key_draft",
    // Section 9's idempotency rule as an index: one subscription, one frozen
    // snapshot, ever. Keyed without the trigger on purpose - D-81 gives the two
    // triggers different instants, so a per-trigger key would let one
    // subscription hold two different frozen prices.
    "uq_pricing_snapshot_provenance_subscription",
];

/// Every named CHECK constraint the chain declares, **by name**.
///
/// A roster and not a count, for `postgres_migrations.rs`'s reason: a count cannot tell
/// a guard from a tautology.
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
    // Slice 10's composite meter. One CHECK only: the arity and self-reference
    // rules are publish rules rather than column constraints -- see
    // `m20260802_000046`'s module doc for why, and `m20260802_000045`'s for the
    // portability argument it inherits.
    "chk_pricing_composite_meter_output_unit",
    "chk_pricing_idempotency_dedup_answered",
    "chk_pricing_idempotency_dedup_status",
    // Slice 11. The two implications that carry section 4's reachable set
    // (`cancelled` is reachable both started and unstarted, so `started_at` is
    // deliberately not a biconditional), the two that are, D-65's co-nullable
    // exclusion set, D-49's row-local half, and three ordering rules.
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
    // Slice 9's seventeen. Note the near-collision one line up:
    // `chk_pricing_price_overlay` is the **price row's** `price_overlay` axis
    // CHECK (always `base`, Foundation section 4.1), and everything from here
    // down belongs to the overlay object — which is a separate row evaluated
    // downstream, not a value of that axis.
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

/// Every trigger's body, pinned by digest — the roster, in `sqlite_master`'s
/// name order.
///
/// A module-level `const` like the four rosters above it, and it was a `Vec`
/// built inside the accessor until Slice 8's nine triggers pushed that function
/// past the line cap. Nothing about it was ever computed: the digests are
/// literals that a legitimate change to a trigger re-pins here, deliberately.
/// Every table's primary key, `(table, "col, col")`, ordered by table name.
///
/// **Seeded from the live schema once (D-236) and then hand-checked**, because a
/// roster taken from the code's own output pins whatever the code does, bug
/// included. Four keys are named by a decision and were each read back against the
/// migration that declares them rather than against the schema this census reads:
/// `pricing_catalog_version_ref` widened to carry the subject beside the handle
/// (`m20260802_000036`, the change D-236 is about — every other roster passed it
/// unchanged); `pricing_read_model`'s matching four-part key
/// (`m20260802_000003`); `pricing_price_overlay_line`'s `(line_id,
/// overlay_revision)` (`m20260802_000033`, D-42's line container); and
/// `pricing_plan`'s `(plan_id, revision)` (`m20260802_000001`, the revision
/// identity D-145 makes a name rather than a counter). The rest were read for
/// shape: every revision-scoped child carries its parent's revision, every
/// tenant-scoped config table is keyed by tenant, and `coord_leases` is `coord`'s
/// own table spliced in for the warm re-drive.
///
/// **A table whose key is empty gets a line saying so**, not a gap. The silence
/// D-236 names is a key absent from the census, so absence must be spelled.
const EXPECTED_PRIMARY_KEYS: &[(&str, &str)] = &[
    ("coord_leases", "key"),
    ("pricing_approval", "approval_id"),
    ("pricing_approval_key", "approval_id, scope_key"),
    ("pricing_approval_threshold", "tenant_id, version, currency"),
    ("pricing_approval_threshold_tombstone", "tenant_id, version"),
    ("pricing_audit_log", "tenant_id, chain_id, seq"),
    ("pricing_brand_taxonomy", "tenant_id, value"),
    ("pricing_bundle", "bundle_id"),
    (
        "pricing_bundle_component",
        "bundle_id, plan_revision, component_plan_id",
    ),
    (
        "pricing_bundle_revshare",
        "bundle_id, plan_revision, vendor_sku_id, party",
    ),
    (
        "pricing_bundle_revshare_group",
        "bundle_id, plan_revision, vendor_sku_id",
    ),
    (
        "pricing_catalog_version_ref",
        "tenant_id, pending_ref, subject_kind, subject_ref",
    ),
    // Slice 10's composite meter, D-106's revision discipline in the key
    // itself: `composite_id` is stable across revisions and the revision is the
    // second column, so a copy-forward is a new row rather than an edit.
    ("pricing_composite_meter", "composite_id, plan_revision"),
    (
        "pricing_idempotency_dedup",
        "tenant_id, operation, client_key",
    ),
    // Client-supplied (`inst-ms-api`, M2): the idempotency key and the primary
    // key are one column, read back from `m20260802_000043`'s own DDL rather
    // than from the schema the census queries.
    ("pricing_migration", "migration_id"),
    ("pricing_operator_flag", "tenant_id, subject_ref, flag"),
    ("pricing_org_tier_taxonomy", "tenant_id, value"),
    ("pricing_outbox", "outbox_id"),
    ("pricing_partner_taxonomy", "tenant_id, value"),
    ("pricing_pin_frontier", "tenant_id"),
    ("pricing_plan", "plan_id, revision"),
    (
        "pricing_plan_addon_rule",
        "plan_id, plan_revision, addon_sku_id",
    ),
    ("pricing_plan_descriptor_set", "plan_id, plan_revision"),
    ("pricing_plan_phase", "phase_id, plan_revision"),
    ("pricing_policy_object", "tenant_id"),
    ("pricing_price", "price_id"),
    ("pricing_price_overlay", "price_overlay_id, revision"),
    ("pricing_price_overlay_line", "line_id, overlay_revision"),
    (
        "pricing_price_overlay_line_amount",
        "line_id, overlay_revision, currency",
    ),
    ("pricing_price_tier_band", "band_id"),
    ("pricing_price_window", "window_id"),
    (
        "pricing_read_model",
        "tenant_id, catalog_version, subject_kind, subject_ref",
    ),
    ("pricing_region_taxonomy", "tenant_id, value"),
    // Read back from `m20260802_000044`'s own DDL. `provenance_id` and not the
    // subscription: the subscription's uniqueness is a partial-free UNIQUE index
    // beside it, which is the rule rather than the identity.
    ("pricing_snapshot_provenance", "provenance_id"),
];

fn expected_primary_keys() -> Vec<(String, String)> {
    EXPECTED_PRIMARY_KEYS
        .iter()
        .map(|(table, columns)| ((*table).to_owned(), (*columns).to_owned()))
        .collect()
}

const EXPECTED_TRIGGER_BODIES: &[(&str, u64)] = &[
    (
        "trg_pricing_approval_born_submitted",
        8_026_324_167_547_094_374_u64,
    ),
    (
        "trg_pricing_approval_flip_whitelist",
        7_582_204_510_596_437_500_u64,
    ),
    (
        "trg_pricing_approval_immutable_once_decided",
        8_082_372_707_353_450_395_u64,
    ),
    (
        "trg_pricing_approval_key_born_submitted",
        2_440_950_770_022_816_954_u64,
    ),
    (
        "trg_pricing_approval_key_born_under_a_pending_unit",
        17_401_364_553_705_688_472_u64,
    ),
    (
        "trg_pricing_approval_key_follow_state",
        1_503_439_957_052_833_582_u64,
    ),
    (
        "trg_pricing_approval_key_follows_its_unit",
        3_659_799_981_708_444_309_u64,
    ),
    (
        "trg_pricing_approval_key_follows_once",
        17_250_957_205_851_589_411_u64,
    ),
    (
        "trg_pricing_approval_key_no_delete",
        8_268_739_358_483_246_584_u64,
    ),
    (
        "trg_pricing_approval_key_pinned_columns",
        2_791_595_470_115_359_269_u64,
    ),
    (
        "trg_pricing_approval_no_delete",
        13_958_316_444_295_959_010_u64,
    ),
    (
        "trg_pricing_approval_pinned_columns",
        16_147_021_889_530_757_421_u64,
    ),
    (
        "trg_pricing_approval_threshold_no_delete",
        12_053_586_872_877_274_445_u64,
    ),
    (
        "trg_pricing_approval_threshold_no_update",
        11_709_629_607_986_505_725_u64,
    ),
    (
        "trg_pricing_approval_threshold_tombstone_no_delete",
        12_721_364_154_841_815_973_u64,
    ),
    (
        "trg_pricing_approval_threshold_tombstone_no_update",
        12_807_063_624_490_211_381_u64,
    ),
    (
        "trg_pricing_audit_log_no_delete",
        4_599_062_756_050_227_754_u64,
    ),
    (
        "trg_pricing_audit_log_no_update",
        8_228_055_037_257_075_408_u64,
    ),
    (
        "trg_pricing_bundle_component_no_delete",
        17_530_640_725_345_118_770_u64,
    ),
    (
        "trg_pricing_bundle_component_no_insert",
        11_007_480_391_378_903_366_u64,
    ),
    (
        "trg_pricing_bundle_component_no_update",
        3_869_036_056_478_704_678_u64,
    ),
    (
        "trg_pricing_bundle_revshare_group_no_delete",
        1_541_310_430_529_535_174_u64,
    ),
    (
        "trg_pricing_bundle_revshare_group_no_insert",
        13_702_565_162_429_317_894_u64,
    ),
    (
        "trg_pricing_bundle_revshare_group_no_update",
        5_081_614_686_098_930_342_u64,
    ),
    (
        "trg_pricing_bundle_revshare_no_delete",
        16_850_888_399_988_183_435_u64,
    ),
    (
        "trg_pricing_bundle_revshare_no_insert",
        17_648_799_137_787_949_339_u64,
    ),
    (
        "trg_pricing_bundle_revshare_no_update",
        12_884_963_950_550_849_015_u64,
    ),
    // Slice 10's composite meter. Read back off the built schema, as this
    // census requires -- a hand-derived number could only ever be this same
    // read-back written down twice.
    (
        "trg_pricing_composite_meter_no_delete",
        4_725_017_676_843_212_914_u64,
    ),
    (
        "trg_pricing_composite_meter_no_insert",
        14_438_003_716_742_837_994_u64,
    ),
    (
        "trg_pricing_composite_meter_no_update",
        2_815_598_062_845_254_393_u64,
    ),
    // Slice 11's five arms. Each digest is the stored body's, which is the only
    // place a digest can come from; what was checked against `m20260802_000043`
    // itself is the **content** each one pins - the DDL text of all five was
    // diffed against the trigger SQL `sqlite_master` holds, so the digests below
    // pin the migration's own statements rather than whatever the chain happened
    // to produce.
    (
        "trg_pricing_migration_exclusion_replay",
        9_285_572_797_681_964_741_u64,
    ),
    (
        "trg_pricing_migration_flip_whitelist",
        8_032_921_964_833_177_687_u64,
    ),
    (
        "trg_pricing_migration_frozen_columns",
        3_679_481_201_681_159_361_u64,
    ),
    (
        "trg_pricing_migration_immutable_history",
        7_463_691_635_803_712_952_u64,
    ),
    (
        "trg_pricing_migration_no_delete",
        759_106_875_609_865_220_u64,
    ),
    (
        "trg_pricing_plan_addon_rule_no_delete",
        16_098_381_818_331_615_522_u64,
    ),
    (
        "trg_pricing_plan_addon_rule_no_insert",
        3_837_286_324_373_750_486_u64,
    ),
    (
        "trg_pricing_plan_addon_rule_no_update",
        16_520_572_091_884_197_445_u64,
    ),
    (
        "trg_pricing_plan_descriptor_set_no_delete",
        7_950_040_988_195_990_411_u64,
    ),
    (
        "trg_pricing_plan_descriptor_set_no_insert",
        6_417_891_462_364_744_011_u64,
    ),
    (
        "trg_pricing_plan_descriptor_set_no_update",
        17_644_945_910_566_070_594_u64,
    ),
    (
        "trg_pricing_plan_draft_flip_whitelist",
        1_063_197_060_918_151_682_u64,
    ),
    (
        "trg_pricing_plan_flip_whitelist",
        2_936_899_670_102_780_293_u64,
    ),
    (
        "trg_pricing_plan_frozen_columns",
        12_503_936_757_081_989_316_u64,
    ),
    ("trg_pricing_plan_no_delete", 11_619_837_810_759_772_588_u64),
    (
        "trg_pricing_plan_phase_no_delete",
        10_318_237_350_173_185_356_u64,
    ),
    (
        "trg_pricing_plan_phase_no_insert",
        5_810_769_608_047_845_284_u64,
    ),
    (
        "trg_pricing_plan_phase_no_update",
        3_878_195_498_503_408_715_u64,
    ),
    (
        "trg_pricing_price_draft_flip_whitelist",
        12_283_433_772_935_461_712_u64,
    ),
    (
        "trg_pricing_price_flip_whitelist",
        6_864_967_922_611_899_704_u64,
    ),
    // Re-pinned twice, and the note is kept whole because the second re-pin
    // rests on the first one's evidence.
    //
    // `m20260802_000040` (`T-18`): the `WHEN` gained `tax_category_ref` and
    // `resolved_tax_category` (digest `6_213_418_472_074_147_039`).
    //
    // `m20260802_000051` (Slice 6): it gained `billing_anchor_policy`,
    // `anchor_day`, `proration_basis` and `credit_on_downgrade`
    // (digest `15_497_072_427_694_831_591`).
    //
    // `m20260802_000055` (Slice 10): it gained `reserved_rate_minor` and
    // `reservation_flavor` (digest `12_168_221_688_212_379_341`). The count in
    // this comment read "71 of 72" for one commit and was wrong: the roster has
    // **74** entries, and a `grep` for the entries at line start missed the two
    // `cargo fmt` had folded onto one line. A filtered count is not a census.
    //
    // `m20260802_000057` (Slice 10): it gained `min_qty_purchase`,
    // `min_qty_usage`, `min_qty_usage_fallback` and `discount_ref` — the other
    // **73** of this roster's 74 entries matched byte for byte. **Exactly one
    // digest moved in this roster on each occasion**, which is the evidence a
    // whole-trigger restatement lost nothing, and the only evidence there is,
    // since neither engine has an incremental form for this check.
    (
        "trg_pricing_price_frozen_columns",
        13_930_095_509_110_238_239_u64,
    ),
    (
        "trg_pricing_price_grandfather_monotonic",
        6_472_678_356_918_752_723_u64,
    ),
    ("trg_pricing_price_no_delete", 4_952_185_589_843_057_617_u64),
    // Slice 9's ten. Hashes read back off the built schema rather than
    // hand-derived: what this census defends is that a `WHEN` clause cannot
    // silently lose a disjunct, and a hand-written number could only ever be
    // this same read-back written down twice.
    // Re-pinned 2026-08-08 by `m20260802_000045` (D-231), which added `abandoned`
    // to the draft's exit list. Was `2_292_452_181_472_169_825`.
    (
        "trg_pricing_price_overlay_draft_exit",
        1_991_170_413_819_745_157_u64,
    ),
    (
        "trg_pricing_price_overlay_frozen_columns",
        17_562_952_219_673_499_162_u64,
    ),
    (
        "trg_pricing_price_overlay_frozen_flip",
        15_942_597_115_297_834_987_u64,
    ),
    (
        "trg_pricing_price_overlay_line_amount_no_delete",
        6_126_575_409_268_361_048_u64,
    ),
    (
        "trg_pricing_price_overlay_line_amount_no_insert",
        14_004_697_873_420_631_368_u64,
    ),
    (
        "trg_pricing_price_overlay_line_amount_no_update",
        10_194_414_681_832_050_017_u64,
    ),
    (
        "trg_pricing_price_overlay_line_no_delete",
        6_741_335_712_440_341_628_u64,
    ),
    (
        "trg_pricing_price_overlay_line_no_insert",
        12_992_281_057_857_453_494_u64,
    ),
    (
        "trg_pricing_price_overlay_line_no_update",
        15_101_858_051_437_048_819_u64,
    ),
    // Re-pinned 2026-08-08 by `m20260802_000045` (D-231): the `WHEN
    // OLD.lifecycle_state <> 'draft'` guard is gone, so the trigger refuses every
    // DELETE rather than every DELETE but a draft's. This is the digest whose
    // movement *is* the fix — the state alone would have left the old exit open.
    // Was `1_755_439_239_709_496_796`.
    (
        "trg_pricing_price_overlay_no_delete",
        11_656_928_464_923_846_849_u64,
    ),
    (
        "trg_pricing_price_tier_band_kind_insert",
        12_169_947_829_544_681_527_u64,
    ),
    (
        "trg_pricing_price_tier_band_kind_update",
        4_931_887_817_904_621_455_u64,
    ),
    (
        "trg_pricing_price_tier_band_no_delete",
        18_080_063_622_350_427_273_u64,
    ),
    (
        "trg_pricing_price_tier_band_no_insert",
        8_466_772_667_392_780_886_u64,
    ),
    (
        "trg_pricing_price_tier_band_no_update",
        17_300_718_464_613_080_632_u64,
    ),
    (
        "trg_pricing_price_tier_band_parent_kind",
        13_015_595_855_638_009_436_u64,
    ),
    (
        "trg_pricing_price_window_act_sequence",
        8_416_459_948_900_544_137_u64,
    ),
    (
        "trg_pricing_price_window_flip_whitelist",
        7_945_364_764_739_140_221_u64,
    ),
    (
        "trg_pricing_price_window_frozen_columns",
        3_006_703_635_329_582_194_u64,
    ),
    (
        "trg_pricing_price_window_future_end",
        674_975_173_687_143_698_u64,
    ),
    (
        "trg_pricing_price_window_immutable_history",
        2_969_521_874_630_654_905_u64,
    ),
    (
        "trg_pricing_price_window_no_delete",
        8_334_934_610_813_099_928_u64,
    ),
    // Slice 11's two. As with `m20260802_000043`'s five, the digest can only come
    // from the stored body; what was checked against the migration itself is the
    // **content** - both statements were diffed byte-for-byte against what
    // `sqlite_master` holds, so these pin the migration's own DDL.
    (
        "trg_pricing_snapshot_provenance_no_delete",
        14_812_364_302_530_093_290_u64,
    ),
    (
        "trg_pricing_snapshot_provenance_no_update",
        3_248_933_936_782_516_701_u64,
    ),
];

/// [`EXPECTED_TRIGGER_BODIES`] in the shape the assertion compares against.
fn expected_trigger_bodies() -> Vec<(String, u64)> {
    EXPECTED_TRIGGER_BODIES
        .iter()
        .map(|(name, hash)| ((*name).to_owned(), *hash))
        .collect()
}

/// Every **named CHECK constraint** the chain created, in name order.
///
/// The census `SQLite` had none of, and it is the backend every in-crate suite runs on.
/// A CHECK is not a `sqlite_master` object of its own — it lives inside its table's
/// `CREATE TABLE` text — so a dropped one leaves the table, the trigger census and the
/// index census all green while the value it refused reaches the column. Two of this
/// chain's migrations **re-type whole table bodies by hand** (`SQLite` cannot
/// `ALTER TABLE ... DROP CONSTRAINT`, so `m20260802_000018` and `m20260802_000019`
/// rebuild), which is exactly the operation a constraint goes missing in.
///
/// Read out of the DDL rather than out of the migration sources on purpose: the
/// question is what the database ended up with.
async fn checks_of(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    let sql = "SELECT sql AS v FROM sqlite_master WHERE type = 'table' \
               AND name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY name";
    let rows = conn
        .query_all(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query sqlite_master");
    let mut names: Vec<String> = Vec::new();
    for row in rows {
        let ddl: String = row.try_get("", "v").expect("the table's DDL");
        // `CONSTRAINT <name> CHECK` is how every one of them is declared; the name is
        // what a refusal message carries, so it is what a census can be written
        // against.
        let mut rest = ddl.as_str();
        while let Some(at) = rest.find("CONSTRAINT ") {
            rest = &rest[at + "CONSTRAINT ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if rest[name.len()..].trim_start().starts_with("CHECK") {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

/// Every table's primary key, as `(table, "col, col")`.
///
/// `PRAGMA table_info`'s `pk` is the **1-based position within the key**, not a
/// boolean, so the columns are ordered by it rather than by their position in the
/// table. A composite key read in declaration order instead of key order is a
/// different key, and this census would then pass a real change.
async fn primary_keys_of(conn: &sea_orm::DatabaseConnection) -> Vec<(String, String)> {
    let tables = objects_of(conn, "table").await;
    let mut keys: Vec<(String, String)> = Vec::new();
    for table in tables {
        let rows = conn
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                format!("PRAGMA table_info({table})"),
            ))
            .await
            .unwrap_or_else(|e| panic!("table_info for {table}: {e}"));
        let mut parts: Vec<(i32, String)> = Vec::new();
        for row in rows {
            let position: i32 = row.try_get("", "pk").expect("the `pk` ordinal");
            if position > 0 {
                parts.push((position, row.try_get("", "name").expect("the column name")));
            }
        }
        parts.sort_by_key(|(position, _)| *position);
        let columns: Vec<String> = parts.into_iter().map(|(_, name)| name).collect();
        // A table with no declared key is recorded as such rather than skipped: the
        // silence D-236 is about is a key that is *absent* from the census, so an
        // empty key must be a line here and not a gap.
        keys.push((table, columns.join(", ")));
    }
    keys
}

/// A stable, dependency-free digest of one string — FNV-1a, 64-bit.
///
/// **Not a security primitive and deliberately not `sha2`** (DE0708 bans it in this
/// crate): what this pins is a transcription, and the adversary is a typo in a
/// hand-rebuilt trigger body rather than a forger.
fn digest(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Every trigger the chain created as `(name, digest-of-body)`, in name order.
///
/// The name census answers "is the trigger there" and **nothing about what it does**.
/// That gap is not hypothetical here: `m20260802_000019` re-types eight trigger bodies
/// by hand, because a `SQLite` table rebuild drops the triggers attached to the old
/// table and they have to be re-created — and a `RAISE(ABORT)` whose `WHEN` clause lost
/// a disjunct is a guard that still exists, still has its name, and refuses less.
async fn trigger_bodies(conn: &sea_orm::DatabaseConnection) -> Vec<(String, u64)> {
    let sql = "SELECT name AS n, sql AS v FROM sqlite_master WHERE type = 'trigger' \
               AND name NOT LIKE 'sqlite_%' ORDER BY name";
    let rows = conn
        .query_all(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query sqlite_master");
    rows.into_iter()
        .map(|row| {
            let name: String = row.try_get("", "n").expect("the trigger's name");
            let body: String = row.try_get("", "v").expect("the trigger's body");
            // Whitespace-normalised, so a reformat that changes no rule does not
            // present as a changed guard.
            let normalised = body.split_whitespace().collect::<Vec<_>>().join(" ");
            (name, digest(&normalised))
        })
        .collect()
}

/// Every object of one `sqlite_master` type the chain created, in name order.
async fn objects_of(conn: &sea_orm::DatabaseConnection, kind: &str) -> Vec<String> {
    let sql = format!(
        "SELECT name AS v FROM sqlite_master WHERE type = '{kind}' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name"
    );
    conn.query_all(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        sql,
    ))
    .await
    .expect("query sqlite_master")
    .iter()
    .map(|row| row.try_get::<String>("", "v").expect("a name"))
    .collect()
}

/// Read every row of an entity under a tenant scope, asserting the table and
/// its column set are what the entity expects.
macro_rules! assert_readable {
    ($conn:expr, $scope:expr, $($entity:path),+ $(,)?) => {
        $(
            let rows = <$entity>::find()
                .secure()
                .scope_with($scope)
                .all($conn)
                .await
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", stringify!($entity)));
            assert!(rows.is_empty(), "{} starts empty", stringify!($entity));
        )+
    };
}

async fn table_exists(conn: &sea_orm::DatabaseConnection, table: &str) -> bool {
    let sql = format!(
        "SELECT count(*) AS c FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
    );
    let row = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("count query returns a row");
    row.try_get::<i32>("", "c").expect("read count") == 1
}

/// The `CREATE INDEX ...` statement `SQLite` recorded for `index`, or `None`
/// when the chain created no index by that name.
async fn index_sql(conn: &sea_orm::DatabaseConnection, index: &str) -> Option<String> {
    let sql = format!(
        "SELECT count(*) AS c FROM sqlite_master WHERE type = 'index' AND name = '{index}'"
    );
    let present = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("count query returns a row")
        .try_get::<i32>("", "c")
        .expect("read count")
        == 1;
    if !present {
        return None;
    }
    let sql =
        format!("SELECT sql AS v FROM sqlite_master WHERE type = 'index' AND name = '{index}'");
    let statement = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("the index row is there")
        .try_get::<String>("", "v")
        .expect("an index this chain created carries its DDL");
    Some(statement)
}

/// The chain, in the order the platform runner applies it (by migration NAME).
fn name_ordered_chain() -> Vec<Box<dyn MigrationTrait>> {
    let mut chain = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    chain
}

#[tokio::test]
async fn the_chain_creates_every_table_and_re_runs_cleanly() {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");

    let boot1 = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 1 must apply the whole chain");
    assert_eq!(
        boot1.applied,
        Migrator::migrations().len(),
        "boot 1 applies every migration"
    );
    assert_eq!(boot1.skipped, 0, "boot 1 skips nothing");

    // Boot 2 over the same database: nothing re-runs. No `CREATE TABLE` in this
    // chain is `IF NOT EXISTS`, so a re-run that actually executed would fail
    // loudly here rather than silently.
    let boot2 = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 2 must be a clean no-op");
    assert_eq!(boot2.applied, 0, "boot 2 applies nothing");
    assert_eq!(
        boot2.skipped,
        Migrator::migrations().len(),
        "boot 2 skips every migration"
    );

    let provider = DBProvider::<DbError>::new(db);
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(Uuid::from_u128(1));
    assert_readable!(
        &conn,
        &scope,
        plan::Entity,
        plan_phase::Entity,
        plan_addon_rule::Entity,
        plan_descriptor_set::Entity,
        price::Entity,
        price_tier_band::Entity,
        read_model::Entity,
        catalog_version_ref::Entity,
        pin_frontier::Entity,
        policy_object::Entity,
        operator_flag::Entity,
        idempotency_dedup::Entity,
        outbox::Entity,
        audit_log::Entity,
        approval::Entity,
        approval_key::Entity,
        approval_threshold::Entity,
        approval_threshold_tombstone::Entity,
        price_window::Entity,
        // Slice 9's seven. This is what proves each entity's column set is the
        // one its migration built — a mismatch is a runtime `SeaORM` error on
        // the first read and nothing earlier catches it.
        region_taxonomy::Entity,
        brand_taxonomy::Entity,
        partner_taxonomy::Entity,
        org_tier_taxonomy::Entity,
        price_overlay::Entity,
        price_overlay_line::Entity,
        price_overlay_line_amount::Entity,
    );
}

#[tokio::test]
async fn the_chain_creates_every_trigger_and_every_index() {
    // The property the whole suite lacked: on `SQLite` a missing guard is invisible
    // to every application-level test, because the rules it enforces are also
    // checked in Rust. `uq_pricing_approval_key_pending` is the sharpest case —
    // delete it and `inst-co-single-pending` degrades from a constraint a concurrent
    // writer cannot step through into a `SELECT` that races, with nothing red.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    assert_eq!(
        objects_of(&conn, "trigger").await,
        EXPECTED_TRIGGERS,
        "the trigger census: a dropped guard, or one added without a line here"
    );
    assert_eq!(
        objects_of(&conn, "index").await,
        EXPECTED_INDEXES,
        "the index census: a dropped rule, or one added without a line here"
    );
    assert_eq!(
        checks_of(&conn).await,
        EXPECTED_CHECKS,
        "the CHECK census: `SQLite` had none, and two migrations re-type whole table bodies by hand"
    );
    assert_eq!(
        primary_keys_of(&conn).await,
        expected_primary_keys(),
        "the primary-key census (D-236): `m20260802_000036` changed the physical identity of a \
         truth-linkage table on the seven-year horizon and every other roster passed unchanged. A \
         key is the one piece of DDL whose loss shows up first as a duplicate row in a table whose \
         whole contract is that it has none"
    );
    assert_eq!(
        trigger_bodies(&conn).await,
        expected_trigger_bodies(),
        "the trigger **body** census: a name census cannot see a `RAISE(ABORT)` whose `WHEN` lost \
         a disjunct, and `m20260802_000019` re-types eight of these by hand. A legitimate change \
         to a trigger re-pins its digest here, deliberately"
    );
}

#[tokio::test]
async fn the_pending_key_register_is_unique_and_partial_on_submitted() {
    // The **shape** and not only the name, because the two halves say different
    // things and only one of them is `inst-co-single-pending`. `UNIQUE` alone would
    // say "one row per key ever", which refuses a second unit over a key whose first
    // unit was decided and withdrawn - the escape `inst-as-void` exists to give. The
    // `WHERE state = 'submitted'` predicate **is** the rule: a decided unit holds
    // nothing.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = index_sql(&conn, "uq_pricing_approval_key_pending")
        .await
        .expect("the register's constraint");
    let upper = ddl.to_ascii_uppercase();
    assert!(upper.contains("UNIQUE"), "one holder per key: {ddl}");
    assert!(
        ddl.contains("(tenant_id, scope_key)"),
        "and the key is per tenant: {ddl}"
    );
    assert!(
        ddl.contains("WHERE state = 'submitted'"),
        "partial on `submitted`, so a decided unit frees its keys: {ddl}"
    );
}

/// D-196 clause (2): both scope-key indexes carry the usage line, **through the
/// sentinel** — and the behaviour, not only the DDL.
///
/// The DDL half is the cheap half. The behavioural half is the one that matters,
/// because the naive widening — listing `meter` itself — produces DDL that reads
/// correct and silently stops constraining every non-usage key, `meter` being
/// nullable and NULLs being distinct inside a `UNIQUE`. So this asserts both
/// directions on the engine the fast gate runs:
///
///   - two usage lines differing only in `meter` are two keys (D-103's example,
///     which the eight-axis key refused);
///   - two rows with **no** meter on one key still collide (the hole).
///
/// The rows go in as raw SQL rather than through the repository on purpose: the
/// repository is the layer that cannot see a guard stop refusing, and until
/// clause (3) it does not carry the pair from the key onto the columns anyway.
#[tokio::test]
async fn both_scope_key_indexes_carry_the_usage_line_through_the_sentinel() {
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    for index in [
        "uq_pricing_price_scope_key_current",
        "uq_pricing_price_scope_key_draft",
    ] {
        let ddl = index_sql(&conn, index).await.expect("the scope-key index");
        assert!(
            ddl.contains("COALESCE(meter, '')"),
            "the meter axis must be indexed through the sentinel, not as the nullable \
             column: {ddl}"
        );
        assert!(
            ddl.contains("dimension_key"),
            "the tenth axis belongs to the key too: {ddl}"
        );
    }

    let row = |id: u32, state: &str, meter: &str| {
        format!(
            "INSERT INTO pricing_price (price_id, tenant_id, plan_id, currency, region, phase, \
             charge_kind, model_kind, amount_minor, lifecycle_state, created_by, created_at_utc, \
             meter) VALUES ('{id:0>8}-0000-0000-0000-000000000000', \
             '11111111-1111-1111-1111-111111111111', '22222222-2222-2222-2222-222222222222', \
             'USD', 'EU', '33333333-3333-3333-3333-333333333333', 'usage', 'per_unit', 1000, \
             '{state}', '44444444-4444-4444-4444-000000000000', '2026-08-03 09:00:00+00', {meter})"
        )
    };
    let exec = async |sql: String| {
        conn.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
    };

    // D-103's example: two meters, one market, two keys — on both planes.
    exec(row(1, "published", "'cloudlets'"))
        .await
        .expect("the first usage line takes its key");
    exec(row(2, "published", "'egress_gb'"))
        .await
        .expect("a second meter is a second key, which is the whole of D-196");
    exec(row(3, "draft", "'cloudlets'"))
        .await
        .expect("the draft plane admits the same pair");
    exec(row(4, "draft", "'egress_gb'"))
        .await
        .expect("a second meter is a second draft key too");

    // And the hole stays closed: no meter is one key, not one key per row.
    exec(row(5, "published", "NULL"))
        .await
        .expect("a meterless usage row takes the sentinel key");
    let collision = exec(row(6, "published", "NULL"))
        .await
        .expect_err("two meterless usage rows share one key and the second must be refused");
    assert!(
        collision
            .to_string()
            .contains("uq_pricing_price_scope_key_current"),
        "the refusal must come from the scope-key index: {collision}"
    );
}

#[tokio::test]
async fn the_policy_mint_guard_is_unique_per_tenant_and_partial_on_both_conjuncts() {
    // The **shape** and not only the name, for the sibling case's reason: each half of
    // this predicate refuses a different legitimate flow if it goes missing, and the
    // name census cannot see either loss.
    //
    // * without `subject_kind = 'policy'` the index would refuse a tenant's second
    //   **plan-revision or window** unit - one tenant holding several of those at once
    //   is the ordinary case, and `inst-co-single-pending` puts that rule on
    //   `pricing_approval_key` per canonical scope key rather than per tenant;
    // * without `state = 'submitted'` it would say "one policy proposal per tenant
    //   **ever**", and a tenant whose first proposal was decided or withdrawn could
    //   never author a second version.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = index_sql(&conn, "uq_pricing_approval_policy_pending")
        .await
        .expect("D-192's mint guard");
    assert!(
        ddl.to_ascii_uppercase().contains("UNIQUE"),
        "one open policy proposal: {ddl}"
    );
    assert!(
        ddl.contains("(tenant_id)"),
        "and it is per tenant, not per deployment: {ddl}"
    );
    assert!(
        ddl.contains("subject_kind = 'policy'"),
        "narrowed to the plane where the rule is per tenant: {ddl}"
    );
    assert!(
        ddl.contains("state = 'submitted'"),
        "partial on `submitted`, so a decided proposal holds nothing: {ddl}"
    );
}

#[tokio::test]
async fn down_then_up_round_trips() {
    // A raw `SeaORM` connection: `SchemaManager` needs one, and the toolkit
    // runner owns bookkeeping but exposes no `down` — this walks the chain the
    // way a rollback would.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let chain = name_ordered_chain();

    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }
    for table in EXPECTED_TABLES {
        assert!(
            table_exists(&conn, table).await,
            "the chain must create `{table}`"
        );
    }

    for migration in chain.iter().rev() {
        migration
            .down(&manager)
            .await
            .unwrap_or_else(|e| panic!("down {} must succeed: {e}", migration.name()));
    }
    for table in EXPECTED_TABLES {
        assert!(
            !table_exists(&conn, table).await,
            "`{table}` must be gone after down"
        );
    }

    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("re-up {} must succeed: {e}", migration.name()));
    }
    // **The second `up` gets the same census as the first.** It used to assert only
    // `EXPECTED_TABLES`, so the second expansion of every `sqlite_rebuild!` — the one a
    // rollback-then-forward actually runs — had no trigger, index or CHECK census at
    // all: a rebuild that dropped a guard on the way back up left this green.
    assert_eq!(
        objects_of(&conn, "trigger").await,
        EXPECTED_TRIGGERS,
        "the re-up must restore every trigger"
    );
    assert_eq!(
        objects_of(&conn, "index").await,
        EXPECTED_INDEXES,
        "the re-up must restore every index"
    );
    assert_eq!(
        checks_of(&conn).await,
        EXPECTED_CHECKS,
        "the re-up must restore every CHECK"
    );
    assert_eq!(
        primary_keys_of(&conn).await,
        expected_primary_keys(),
        "the re-up must restore every primary key. This is the direction D-236 names as equally \
         silent: a rebuild that restates a table and drops a key column, which no \
         `sqlite_rebuild!` expansion was checked for before this roster existed"
    );
    assert_eq!(
        trigger_bodies(&conn).await,
        expected_trigger_bodies(),
        "and restore them with the same bodies, not merely the same names"
    );
    for table in EXPECTED_TABLES {
        assert!(
            table_exists(&conn, table).await,
            "`{table}` must be back after the re-up"
        );
    }
}

#[tokio::test]
async fn the_version_index_on_the_ref_table_is_not_unique() {
    // The amendment of 2026-08-03, pinned so it cannot regress into the shape
    // it replaced. `uq_pricing_catalog_version_ref_version` asserted a
    // bijection from committed version to publish — and under the registry's
    // batching (D-47, §4.2 step 5) several of one tenant's pending refs commit
    // into ONE version, which is the case the whole model exists to serve and
    // which that index made physically impossible: the second finalize failed.
    // D-157's subject columns already answer "which publish produced this
    // version", and the honest answer is a set.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    assert_eq!(
        index_sql(&conn, "uq_pricing_catalog_version_ref_version").await,
        None,
        "the unique version index is gone: it refused D-47's normal case"
    );
    let created = index_sql(&conn, "idx_pricing_catalog_version_ref_version")
        .await
        .expect("the version index the projector and the frontier walk read");
    assert!(
        !created.to_ascii_uppercase().contains("UNIQUE"),
        "the replacement must be non-unique, got: {created}"
    );
}

#[tokio::test]
async fn the_ref_tables_key_carries_the_subject_as_well_as_the_handle() {
    // `m20260802_000036` (D-234), pinned **because no roster covers it**. This
    // suite carries five rosters — tables, triggers, indexes, CHECK names and
    // trigger bodies — and a primary key is none of them, so the chain changing
    // the physical identity of a truth-linkage table passed every store census
    // in this file silently. That gap is general and is recorded as such; this
    // test closes it for the one table the change was about.
    //
    // What it pins: a handle names one registry assignment, and a publish unit
    // records against it every subject it projects — one on the plan plane, two
    // on the overlay plane and three when a revision moves the scope value
    // (D-112, D-133). Narrowed back to the handle, the second subject of an
    // overlay publish is refused by the key and the act cannot commit at all.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = table_sql(&conn, "pricing_catalog_version_ref").await;
    let normalised = ddl.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalised.contains("PRIMARY KEY (tenant_id, pending_ref, subject_kind, subject_ref)"),
        "the key must carry the subject, got: {normalised}"
    );
}

/// The `CREATE TABLE ...` statement `SQLite` recorded for `table`.
async fn table_sql(conn: &sea_orm::DatabaseConnection, table: &str) -> String {
    let sql =
        format!("SELECT sql AS v FROM sqlite_master WHERE type = 'table' AND name = '{table}'");
    conn.query_one(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        sql,
    ))
    .await
    .expect("query sqlite_master")
    .expect("the table row is there")
    .try_get::<String>("", "v")
    .expect("a table this chain created carries its DDL")
}

#[tokio::test]
async fn the_ref_table_records_the_commit_observation_and_pairs_it_with_nothing() {
    // D-166 clause (1). The column is what every post-commit clause in the set
    // was written against and none of them had: `requested_at` measures the
    // batching wait the requirement explicitly puts OUTSIDE degraded handling,
    // and `committed_at` is stamped by the finalize, which is the step that
    // never runs on the path the signal exists for.
    //
    // The absence of a CHECK is the assertion's other half.
    // `chk_pricing_catalog_version_ref_commit` exists because a version and its
    // commit instant are one fact; this column's whole purpose is to be settable
    // while `catalog_version` is still NULL.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = table_sql(&conn, "pricing_catalog_version_ref").await;
    assert!(
        ddl.contains("commit_observed_at"),
        "the SQLite arm must create the column: {ddl}"
    );
    assert!(
        !ddl.replace("commit_observed_at text", "")
            .contains("commit_observed_at"),
        "and pair it with nothing - no CHECK may mention it: {ddl}"
    );
}

/// D-240: `tax_display_mode` is retired, and the rebuild that retires it kept
/// everything else `pricing_policy_object` carries.
///
/// The column is dropped by a create-copy-drop-rename, because `SQLite` refuses
/// to drop a column a CHECK names. That makes the *retained* half the assertion
/// that matters: a rebuild silently omitting an arm is the failure this shape
/// has, and it is invisible to a case that only checks the column is gone. So
/// every surviving column and every surviving CHECK is named here rather than
/// counted — a count passes against a rebuild that dropped one arm and grew
/// another.
///
/// `tax_display_policy_mode` is listed among them deliberately. It is the
/// column the rebuild is most likely to lose: `m20260802_000038` added it with
/// a plain `ALTER TABLE … ADD COLUMN` **after** `m20260802_000018` last restated
/// this table, so a rebuild written from `000018`'s text alone — the natural
/// mistake, and the one the brief for this work flagged first — would drop C4's
/// fail-closed switch and still look complete.
#[tokio::test]
async fn the_retired_tax_display_mode_leaves_every_other_policy_column_standing() {
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = table_sql(&conn, "pricing_policy_object").await;

    assert!(
        !ddl.contains("tax_display_mode"),
        "D-240 retires the column, so no arm of the table may still name it: {ddl}"
    );
    assert!(
        !ddl.contains("chk_pricing_policy_object_tax_display CHECK"),
        "and the CHECK that guarded it goes with it: {ddl}"
    );

    for column in [
        "tenant_id",
        "default_rounding_policy_ref",
        "enforced_migration_notice_days",
        "max_tier_bands_per_row",
        "max_price_rows_per_plan",
        "max_custom_interval_days",
        "max_custom_interval_months",
        "additional_required_descriptors",
        "updated_at_utc",
        "updated_by",
        "tax_display_policy_mode",
    ] {
        assert!(
            ddl.contains(column),
            "the rebuild must carry `{column}` across: {ddl}"
        );
    }

    for check in [
        "chk_pricing_policy_object_notice_floor",
        "chk_pricing_policy_object_tier_band_cap",
        "chk_pricing_policy_object_price_row_cap",
        "chk_pricing_policy_object_interval_days_cap",
        "chk_pricing_policy_object_interval_months_cap",
        "chk_pricing_policy_object_tax_display_policy",
    ] {
        assert!(
            ddl.contains(check),
            "the rebuild must carry `{check}` across: {ddl}"
        );
    }
}
