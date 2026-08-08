//! Migration set for the bss-pricing gear (schema `bss`).
//!
//! Greenfield chain: the ten Foundation-owned tables of
//! `design/01-foundation.md` §3.7, one migration per table, in dependency
//! order, then the slice-owned tables — each of which names its slice, its §6 and
//! its reason for existing in **its own** module doc.
//!
//! **The per-slice enumeration that stood here is gone rather than extended**, and the
//! reason is what happened to it: it listed Slices 2, 3, 5 and 7's tables and then
//! silently stopped being the whole set when `pricing_approval_key` was appended, so
//! a reader who trusted it was told the register did not exist. `Migrator::migrations`
//! below is the roster; a second copy of it in prose is a second thing to keep true,
//! and the copy is the one that goes stale.
//!
//! Two shape rules the roster does not show, which is why they are worth prose:
//! **a table per migration**, so the chain reads as a list of decisions rather than a
//! script; and **columns a slice adds to an existing table are amended onto it**
//! rather than tabled separately — Slice 3's per-row price columns onto
//! `pricing_price`, Slice 2's per-revision columns onto `pricing_plan` — because a
//! side table keyed the same way as its parent is a join nobody needs and a second
//! place for one row's facts to live.
//!
//! **Schema creation.** The chain has no separate `create_bss_schema`
//! migration: `m20260802_000001_create_pricing_plan` issues
//! `CREATE SCHEMA IF NOT EXISTS bss` as its first Postgres statement, and the
//! shared `coord` lease migration (whose `m0001_...` name sorts first under the
//! toolkit runner's name ordering) issues the same statement before its own
//! `CREATE TABLE`. Both are `IF NOT EXISTS`, so the schema exists no matter
//! which of them the runner reaches first.
//!
//! **Ordering.** The toolkit migration runner applies migrations in **name**
//! order, not vec order, and rejects a duplicate `DeriveMigrationName` outright
//! — which is what `tests/module_test.rs` asserts about this list, because a
//! duplicate name would otherwise be a migration that silently never runs.

pub mod m20260802_000001_create_pricing_plan;
pub mod m20260802_000002_create_pricing_price;
pub mod m20260802_000003_create_pricing_read_model;
pub mod m20260802_000004_create_pricing_catalog_version_ref;
pub mod m20260802_000005_create_pricing_pin_frontier;
pub mod m20260802_000006_create_pricing_policy_object;
pub mod m20260802_000007_create_pricing_operator_flag;
pub mod m20260802_000008_create_pricing_idempotency_dedup;
pub mod m20260802_000009_create_pricing_outbox;
pub mod m20260802_000010_create_pricing_audit_log;
pub mod m20260802_000011_create_pricing_price_tier_band;
pub mod m20260802_000012_create_pricing_plan_phase;
pub mod m20260802_000013_create_pricing_plan_addon_rule;
pub mod m20260802_000014_create_pricing_plan_descriptor_set;
pub mod m20260802_000015_create_pricing_approval;
pub mod m20260802_000016_create_pricing_price_window;
pub mod m20260802_000017_create_pricing_approval_key;
pub mod m20260802_000018_create_pricing_approval_threshold;
pub mod m20260802_000019_widen_pricing_approval_subject_kind;
pub mod m20260802_000020_create_pricing_approval_threshold_tombstone;
pub mod m20260802_000021_add_pricing_price_window_mutation_seq;
pub mod m20260802_000022_guard_one_open_policy_proposal;
pub mod m20260802_000023_widen_pricing_price_scope_key_indexes;
pub mod m20260802_000024_create_pricing_bundle;
pub mod m20260802_000025_create_pricing_bundle_component;
pub mod m20260802_000026_create_pricing_bundle_revshare_group;
pub mod m20260802_000027_create_pricing_bundle_revshare;
pub mod m20260802_000028_create_pricing_region_taxonomy;
pub mod m20260802_000029_create_pricing_brand_taxonomy;
pub mod m20260802_000030_create_pricing_partner_taxonomy;
pub mod m20260802_000031_create_pricing_org_tier_taxonomy;
pub mod m20260802_000032_create_pricing_price_overlay;
pub mod m20260802_000033_create_pricing_price_overlay_line;
pub mod m20260802_000034_create_pricing_price_overlay_line_amount;
pub mod m20260802_000035_widen_pricing_approval_subject_kind_overlay;
pub mod m20260802_000036_widen_pricing_catalog_version_ref_subject_key;
pub mod m20260802_000037_add_pricing_price_tax_category_ref;
pub mod m20260802_000038_add_pricing_policy_object_tax_display_policy;
pub mod m20260802_000039_add_pricing_price_resolved_tax_category;
pub mod m20260802_000040_guard_pricing_price_tax_columns;
pub mod m20260802_000041_retire_pricing_policy_object_tax_display_mode;
pub mod m20260802_000042_tighten_taxonomy_value_present_check;
pub mod m20260802_000043_create_pricing_migration;
pub mod m20260802_000044_create_pricing_snapshot_provenance;
pub mod m20260802_000045_add_price_overlay_abandoned_state;
pub mod m20260802_000046_create_pricing_composite_meter;
pub mod m20260802_000047_create_pricing_bulk_operation;
pub mod m20260802_000048_create_pricing_repricing_journal;
pub mod m20260802_000049_create_pricing_bulk_row_lock;
pub mod m20260802_000050_add_pricing_price_proration_contract;
pub mod m20260802_000051_guard_pricing_price_proration_columns;
pub mod m20260802_000052_add_pricing_plan_change_contract;
pub mod m20260802_000053_add_pricing_plan_entitlement_grants;
pub mod m20260802_000054_add_pricing_price_reservation;
pub mod m20260802_000055_guard_pricing_price_reservation_columns;
pub mod m20260802_000056_add_pricing_price_floor_and_discount;
pub mod m20260802_000057_guard_pricing_price_floor_and_discount_columns;
pub mod m20260802_000058_guard_pricing_plan_later_columns;
pub mod m20260802_000060_add_price_overlay_published_event_name;
pub mod m20260802_000061_add_pricing_plan_cloned_from;
pub mod m20260802_000062_guard_pricing_plan_cloned_from;

use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// Run the backend's statement list, in order.
///
/// Every migration in this chain is a pair of raw-SQL const arrays (Postgres
/// canonical, `SQLite` mirror) plus this dispatch. Factored out rather than
/// copied ten times: the branch that must never drift is "which backend gets
/// which statements", and there is exactly one copy of it.
///
/// # Errors
/// [`DbErr::Migration`] for `MySQL` (not a supported backend for this gear),
/// or the driver's error for a failing statement.
pub(crate) async fn exec_backend(
    manager: &SchemaManager<'_>,
    postgres: &[&str],
    sqlite: &[&str],
) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let conn = manager.get_connection();
    let statements: &[&str] = match backend {
        sea_orm::DatabaseBackend::Postgres => postgres,
        sea_orm::DatabaseBackend::Sqlite => sqlite,
        sea_orm::DatabaseBackend::MySql => {
            return Err(DbErr::Migration(
                "MySQL is not supported for bss-pricing".to_owned(),
            ));
        }
    };
    for sql in statements {
        conn.execute(Statement::from_string(backend, (*sql).to_owned()))
            .await?;
    }
    Ok(())
}

/// The gear's migration chain.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260802_000001_create_pricing_plan::Migration),
            Box::new(m20260802_000002_create_pricing_price::Migration),
            Box::new(m20260802_000003_create_pricing_read_model::Migration),
            Box::new(m20260802_000004_create_pricing_catalog_version_ref::Migration),
            Box::new(m20260802_000005_create_pricing_pin_frontier::Migration),
            Box::new(m20260802_000006_create_pricing_policy_object::Migration),
            Box::new(m20260802_000007_create_pricing_operator_flag::Migration),
            Box::new(m20260802_000008_create_pricing_idempotency_dedup::Migration),
            Box::new(m20260802_000009_create_pricing_outbox::Migration),
            Box::new(m20260802_000010_create_pricing_audit_log::Migration),
            Box::new(m20260802_000011_create_pricing_price_tier_band::Migration),
            Box::new(m20260802_000012_create_pricing_plan_phase::Migration),
            Box::new(m20260802_000013_create_pricing_plan_addon_rule::Migration),
            Box::new(m20260802_000014_create_pricing_plan_descriptor_set::Migration),
            Box::new(m20260802_000015_create_pricing_approval::Migration),
            Box::new(m20260802_000016_create_pricing_price_window::Migration),
            Box::new(m20260802_000017_create_pricing_approval_key::Migration),
            Box::new(m20260802_000018_create_pricing_approval_threshold::Migration),
            Box::new(m20260802_000019_widen_pricing_approval_subject_kind::Migration),
            Box::new(m20260802_000020_create_pricing_approval_threshold_tombstone::Migration),
            Box::new(m20260802_000021_add_pricing_price_window_mutation_seq::Migration),
            Box::new(m20260802_000022_guard_one_open_policy_proposal::Migration),
            Box::new(m20260802_000023_widen_pricing_price_scope_key_indexes::Migration),
            Box::new(m20260802_000024_create_pricing_bundle::Migration),
            Box::new(m20260802_000025_create_pricing_bundle_component::Migration),
            Box::new(m20260802_000026_create_pricing_bundle_revshare_group::Migration),
            Box::new(m20260802_000027_create_pricing_bundle_revshare::Migration),
            // Slice 4's four scope-value taxonomies, created on the Slice 9
            // chain because `inst-plv-scope` validates every non-`global`
            // overlay scope value against one of them and none of the four
            // existed. `m20260802_000028`'s module doc carries the argument.
            Box::new(m20260802_000028_create_pricing_region_taxonomy::Migration),
            Box::new(m20260802_000029_create_pricing_brand_taxonomy::Migration),
            Box::new(m20260802_000030_create_pricing_partner_taxonomy::Migration),
            Box::new(m20260802_000031_create_pricing_org_tier_taxonomy::Migration),
            // Slice 9's three: the overlay header and its revision chain, the
            // D-42 adjustment lines, and the D-08 per-currency values. In
            // dependency order — each one's foreign key points at the one above.
            Box::new(m20260802_000032_create_pricing_price_overlay::Migration),
            Box::new(m20260802_000033_create_pricing_price_overlay_line::Migration),
            Box::new(m20260802_000034_create_pricing_price_overlay_line_amount::Migration),
            // D-158's enumeration gains `price_overlay`, because the overlay plane
            // now writes audit records and `AuditSubjectKind` spells both stores.
            // It rebuilds `pricing_approval` on `SQLite`, so it sorts after every
            // migration that attaches anything to that table — `000022`'s index is
            // the object it restates that `m20260802_000019` did not have to.
            Box::new(m20260802_000035_widen_pricing_approval_subject_kind_overlay::Migration),
            Box::new(m20260802_000036_widen_pricing_catalog_version_ref_subject_key::Migration),
            // **Slice 4's three, renumbered at the merge.** They were authored
            // `000036`…`000038` on `slice/04-currency-tax`, against a base that did
            // not yet carry D-234's `000036` above; both strands took the next free
            // number from the same base and landed on it. The runner orders by
            // **name**, so two `000036`s are not a duplicate it would reject — the
            // names differ and the tables are disjoint — but the number stops being
            // the thing that says what runs when, which is the only job it has.
            // Their order among themselves is unchanged, which is what keeps the
            // column order the strand's roster suites measured.
            Box::new(m20260802_000037_add_pricing_price_tax_category_ref::Migration),
            Box::new(m20260802_000038_add_pricing_policy_object_tax_display_policy::Migration),
            Box::new(m20260802_000039_add_pricing_price_resolved_tax_category::Migration),
            // `T-18`: the frozen-column guard gains the two tax columns, and it
            // has to sort after both `ALTER`s that create them — a trigger
            // naming a column the table does not have yet does not create.
            Box::new(m20260802_000040_guard_pricing_price_tax_columns::Migration),
            // `D-240`: `tax_display_mode` is retired. It rebuilds
            // `pricing_policy_object` on `SQLite`, so it sorts after every
            // migration that puts anything on that table — `000038`'s column is
            // the one it restates that `m20260802_000018`'s rebuild did not have.
            Box::new(m20260802_000041_retire_pricing_policy_object_tax_display_mode::Migration),
            // `D-242`: the taxonomy `value` CHECK stops admitting whitespace. It
            // rebuilds all four taxonomy tables on `SQLite`, so it sorts after
            // `000028`…`000031` that create them; nothing since has restated any
            // of the four, which is why each rebuild is written from its
            // creating migration rather than from a later one.
            Box::new(m20260802_000042_tighten_taxonomy_value_present_check::Migration),
            // Slice 11's migration plane: the schedule row and its Section 4 state
            // machine. It creates a table nothing else in the chain touches, so it
            // restates nothing and sorts anywhere after the plans it references by
            // id - which is every position, since it carries no foreign key (see
            // its module doc for why neither plan reference is expressible as one).
            Box::new(m20260802_000043_create_pricing_migration::Migration),
            // Slice 11's synthesis half: the `migrated-origin` record. Like the
            // table above it restates nothing and carries no foreign key, so its
            // position in the chain is free.
            Box::new(m20260802_000044_create_pricing_snapshot_provenance::Migration),
            // D-231's tombstone: `abandoned` joins the overlay's lifecycle and
            // `DELETE` stops being an exit. It sorts after `000032` creates the
            // table and rewrites that migration's `CHECK` and trigger rather than
            // editing it, for `m20260802_000040`'s reason.
            Box::new(m20260802_000045_add_price_overlay_abandoned_state::Migration),
            // Slice 10's derived-meter definition: one output unit and a formula
            // over >= 2 constituents, revision-scoped and copied forward with a
            // stable `composite_id` (D-106). Sorts after `000001` creates the
            // parent revision its FK names.
            Box::new(m20260802_000046_create_pricing_composite_meter::Migration),
            // Slice 12's bulk-operation record and its state machine. Revision
            // independent, so it owes no copy-forward -- see its module doc.
            Box::new(m20260802_000047_create_pricing_bulk_operation::Migration),
            // Slice 12's per-row idempotency spine. Sorts after `000047` creates
            // the run its FK names and after `000002` creates the price rows its
            // other two keys name.
            Box::new(m20260802_000048_create_pricing_repricing_journal::Migration),
            // Slice 12's bulk lock, a side table rather than a column on
            // `pricing_price` -- the append-only trigger's whitelist refuses the
            // marker on a published row. Sorts after `000047` creates the run its
            // FK names and whose state its INSERT arm reads, and after `000002`
            // creates the price rows its other key names.
            Box::new(m20260802_000049_create_pricing_bulk_row_lock::Migration),
            // Slice 6's proration input contract: four columns on `pricing_price`
            // (`inst-pi-required`). Plain `ALTER`s, so they sort anywhere after
            // `000002` creates the table; the number is this strand's allotted
            // range (`000050`-`000059`), disjoint from the concurrent strand's.
            Box::new(m20260802_000050_add_pricing_price_proration_contract::Migration),
            // The guard restatement for the four columns `000050` adds. It sorts
            // after them because a trigger naming a column that does not exist
            // yet does not create -- `m20260802_000040`'s reason for being its
            // own migration rather than an edit to `m20260802_000002`.
            Box::new(m20260802_000051_guard_pricing_price_proration_columns::Migration),
            // Slice 6's plan-change contract: three columns on `pricing_plan`
            // (`inst-pc-targets` / `inst-pc-rank` / `inst-pc-counter-carry`).
            Box::new(m20260802_000052_add_pricing_plan_change_contract::Migration),
            // Slice 6's entitlement grant set (D-41). One column, not
            // `pricing_plan_grant` -- that is Slice 10's table for D-43's
            // prepaid credit grant, a different object; see this migration's
            // own doc.
            Box::new(m20260802_000053_add_pricing_plan_entitlement_grants::Migration),
            // Slice 10's reserved-capacity attributes: two columns on
            // `pricing_price` (`inst-rv-attrs`, A1). Plain `ALTER`s, so they sort
            // anywhere after `000002`; the number is this strand's allotted range
            // (`000054`-`000059`), disjoint from the concurrent D-231 work's.
            Box::new(m20260802_000054_add_pricing_price_reservation::Migration),
            // The guard restatement for the two columns `000054` adds --
            // `000051`'s shape, and after them for `000051`'s reason: a trigger
            // naming a column that does not exist yet does not create.
            Box::new(m20260802_000055_guard_pricing_price_reservation_columns::Migration),
            // Slice 10's typed floors and the `discountRef` hook: four columns
            // on `pricing_price` (`inst-ft-typed` / `inst-ft-fallback` /
            // `inst-dr-referential`), plus their guard restatement after them
            // for `000051`'s reason.
            Box::new(m20260802_000056_add_pricing_price_floor_and_discount::Migration),
            Box::new(m20260802_000057_guard_pricing_price_floor_and_discount_columns::Migration),
            // `pricing_plan`'s frozen-column guard gains the four columns
            // `000052` and `000053` added and neither restated. Sorts after both,
            // for `m20260802_000040`'s reason: a trigger naming a column that does
            // not exist yet does not create.
            Box::new(m20260802_000058_guard_pricing_plan_later_columns::Migration),
            Box::new(m20260802_000060_add_price_overlay_published_event_name::Migration),
            // Slice 12's clone provenance, and the guard that freezes it. Two
            // migrations rather than one, per `m20260802_000040`: a trigger
            // naming a column that does not exist yet does not create.
            Box::new(m20260802_000061_add_pricing_plan_cloned_from::Migration),
            Box::new(m20260802_000062_guard_pricing_plan_cloned_from::Migration),
            // Shared `coord_leases` table, owned by the `coord` crate. This gear's
            // background work is coordinated as a singleton (§3.8: background work
            // is coordinated as a singleton via the coordination lease library),
            // so it needs the lease table the same way the sibling ledger needs it
            // for its recognition run — one row per leased ticker, each on its own
            // key. Qualified into `bss` so it lands
            // there regardless of the connection's `search_path` order; its
            // `m0001_...` name sorts FIRST under the runner's name ordering, so
            // its `up` creates the schema itself before the `CREATE TABLE`.
            Box::new(coord::migration::Migration::in_schema("bss")),
        ]
    }
}
