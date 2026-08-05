//! The per-tenant authoring policy, from the row all the way to the rule that
//! enforces it (D-152) — against a real database, not a mock.
//!
//! Two things are worth proving here and neither can be proved one layer up.
//! The first is the clause that keeps the ratified numbers from moving: a tenant
//! with **no** policy row is governed by the launch values, and a mock returning
//! a hand-built policy would assert that promise against a value the test itself
//! wrote. The second is that a cap and a required descriptor key travel from the
//! row into the pipeline: the caps are held as rule **fields**, so a resolution
//! that read the right row and then built the rule from the deployment default
//! would look correct at every layer and reject nothing a tenant configured.
//!
//! The pipeline is run whole rather than the two rules called directly. A rule
//! that resolved correctly and was never registered enforces nothing, and the
//! registration is the half a policy read cannot see.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;

use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::plan_rules::{
    DESCRIPTOR_INCOMPLETE, INVALID_CUSTOM_INTERVAL, plan_shape_rules,
};
use bss_pricing::domain::plan_shape::{CustomIntervalUnit, DescriptorSet, Frequency, PlanShape};
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::entity::policy_object;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{AuthoringPolicy, PolicyObjectRepo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::EntityTrait;
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureInsertExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

mod common;

/// The repository over the ratified deployment defaults, plus the provider the
/// seeding helper needs: `pricing_policy_object` has no writer in this gear (a
/// policy change is an approval-workflow unit, not a row write), so a test that
/// wants a configured tenant puts the row there itself.
async fn harness() -> (PolicyObjectRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (
        PolicyObjectRepo::new(provider.clone(), &LimitsConfig::default()),
        provider,
    )
}

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap()
}

/// Write one tenant's policy row, carrying only what a test configures.
///
/// Everything the test does not name is left `NotSet`, so the column defaults
/// the migration declares are the ones exercised rather than values this file
/// re-states.
async fn seed(provider: &DBProvider<DbError>, tenant_id: Uuid, caps: policy_object::ActiveModel) {
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(tenant_id);
    let row = policy_object::ActiveModel {
        tenant_id: Set(tenant_id),
        tax_display_mode: Set("tax_exclusive".to_owned()),
        default_rounding_policy_ref: Set(None),
        enforced_migration_notice_days: Set(60),
        updated_at_utc: Set(at()),
        updated_by: Set(Uuid::from_u128(0xad_11)),
        ..caps
    };
    policy_object::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&scope, &row)
        .expect("scope the policy row")
        .exec(&conn)
        .await
        .expect("insert the policy row");
}

/// A draft carrying a complete v1 descriptor set and one custom interval — the
/// two things this suite's policy entries govern.
fn draft(n: u32, additional: BTreeMap<String, String>) -> PlanShape {
    let mut shape = PlanShape::new(PlanId::new(Uuid::from_u128(0x91a4)), 3, at());
    shape.frequency = Some(Frequency::CustomEveryN {
        n,
        unit: CustomIntervalUnit::Days,
    });
    shape.descriptor_set = Some(DescriptorSet {
        invoice_line_template: Some("Subscription: {plan}".to_owned()),
        gl_code: Some("4000".to_owned()),
        itemization_rule: Some("per_plan".to_owned()),
        additional,
    });
    shape
}

/// Run the whole Slice-2 pipeline built from `policy`, and count one code.
///
/// Counted rather than asserted-empty: a bare `PlanShape` fails several rules
/// that have nothing to do with a policy entry, and a suite that demanded a
/// clean report would be asserting the state of every other rule in the slice.
fn findings(policy: &AuthoringPolicy, shape: &PlanShape, code: &str) -> usize {
    plan_shape_rules(policy.interval_bounds(), policy.descriptor_rule())
        .run(shape)
        .violations
        .iter()
        .filter(|violation| violation.code == code)
        .count()
}

/// The clause that keeps D-152 from moving any ratified number: no row means the
/// launch values, and they reach the rule that enforces them.
#[tokio::test]
async fn a_tenant_with_no_policy_entry_is_governed_by_the_ratified_defaults() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0xd0_01);
    let policy = repo
        .authoring_policy(&AccessScope::for_tenant(tenant), tenant)
        .await
        .expect("read the authoring policy");

    let limits = LimitsConfig::default();
    assert_eq!(
        policy.max_tier_bands_per_row(),
        limits.max_tier_bands_per_row
    );
    assert_eq!(
        policy.max_price_rows_per_plan(),
        limits.max_price_rows_per_plan
    );
    assert!(policy.additional_required_descriptors().is_empty());

    // 366 days is the ratified cap, so 366 passes and 367 does not.
    assert_eq!(
        findings(
            &policy,
            &draft(366, BTreeMap::new()),
            INVALID_CUSTOM_INTERVAL
        ),
        0
    );
    assert_eq!(
        findings(
            &policy,
            &draft(367, BTreeMap::new()),
            INVALID_CUSTOM_INTERVAL
        ),
        1
    );
    // And the descriptor rule is D-48's pinned three, all of them satisfied.
    assert_eq!(
        findings(&policy, &draft(30, BTreeMap::new()), DESCRIPTOR_INCOMPLETE),
        0
    );
}

/// The cap the tenant configured is the cap the rule enforces — including one
/// **below** the ratified default, which is the direction a deployment-wide
/// value could never express for one tenant.
#[tokio::test]
async fn a_configured_cap_reaches_the_rule_that_enforces_it() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0xd0_02);
    seed(
        &provider,
        tenant,
        policy_object::ActiveModel {
            max_custom_interval_days: Set(Some(30)),
            ..Default::default()
        },
    )
    .await;

    let policy = repo
        .authoring_policy(&AccessScope::for_tenant(tenant), tenant)
        .await
        .expect("read the authoring policy");

    // 45 days is inside the ratified 366 and outside this tenant's 30.
    assert_eq!(
        findings(
            &policy,
            &draft(45, BTreeMap::new()),
            INVALID_CUSTOM_INTERVAL
        ),
        1
    );
    assert_eq!(
        findings(
            &policy,
            &draft(30, BTreeMap::new()),
            INVALID_CUSTOM_INTERVAL
        ),
        0
    );

    // Per column, not per row: configuring one cap did not move the other three.
    let limits = LimitsConfig::default();
    assert_eq!(
        policy.max_tier_bands_per_row(),
        limits.max_tier_bands_per_row
    );
    assert_eq!(
        policy.max_price_rows_per_plan(),
        limits.max_price_rows_per_plan
    );
    assert!(policy.additional_required_descriptors().is_empty());
}

/// P5 / `inst-ds-sufficient` end to end: a tenant declares a fourth required
/// descriptor key and the **existing** `DESCRIPTOR_INCOMPLETE` blocks the
/// publish. No new wire code, no migration, no new column.
#[tokio::test]
async fn an_extended_required_key_blocks_publish_through_the_existing_code() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0xd0_03);
    seed(
        &provider,
        tenant,
        policy_object::ActiveModel {
            additional_required_descriptors: Set(json!(["costCentre"])),
            ..Default::default()
        },
    )
    .await;

    let policy = repo
        .authoring_policy(&AccessScope::for_tenant(tenant), tenant)
        .await
        .expect("read the authoring policy");
    assert_eq!(policy.additional_required_descriptors(), ["costCentre"]);

    // A v1-complete set is no longer complete for this tenant.
    assert_eq!(
        findings(&policy, &draft(30, BTreeMap::new()), DESCRIPTOR_INCOMPLETE),
        1
    );

    // And the key is satisfied from `additional_fields`, which is what makes the
    // required set extensible without a schema change.
    let carried = BTreeMap::from([("costCentre".to_owned(), "emea-ops".to_owned())]);
    assert_eq!(
        findings(&policy, &draft(30, carried), DESCRIPTOR_INCOMPLETE),
        0
    );
}

/// SQL-level BOLA. Absence resolves to the deployment defaults rather than to
/// someone else's limits, which is the safe direction: the alternative is
/// enforcing one tenant's caps on another tenant's catalog.
#[tokio::test]
async fn a_foreign_tenants_policy_is_invisible_and_resolves_to_the_defaults() {
    let (repo, provider) = harness().await;
    let mine = Uuid::from_u128(0xd0_11);
    let theirs = Uuid::from_u128(0xd0_22);
    seed(
        &provider,
        theirs,
        policy_object::ActiveModel {
            max_custom_interval_days: Set(Some(7)),
            additional_required_descriptors: Set(json!(["costCentre"])),
            ..Default::default()
        },
    )
    .await;

    let policy = repo
        .authoring_policy(&AccessScope::for_tenant(mine), theirs)
        .await
        .expect("read the authoring policy");

    assert!(policy.additional_required_descriptors().is_empty());
    assert_eq!(
        findings(
            &policy,
            &draft(45, BTreeMap::new()),
            INVALID_CUSTOM_INTERVAL
        ),
        0,
        "their 7-day cap must not govern my catalog"
    );
}

/// Every cap column carries the positivity guard, and the guard is physical: a
/// zero band or row cap makes every plan unpublishable and a zero interval cap
/// makes every custom frequency unpublishable, which is a cap that rejects
/// everything while looking exactly like one that is switched on.
#[tokio::test]
async fn every_cap_column_refuses_a_non_positive_value() {
    // A raw connection: `DbConn` exposes no statement API by design, and the
    // subject here is the DDL rather than anything a repository does with it.
    let conn = common::migrated_db().await;
    let tenant = Uuid::from_u128(0xd0_55);

    // Each pair is asserted by its own constraint name, not by "some error
    // happened": four columns sharing one refusal would let three of the guards
    // be dropped with the suite still green.
    for (column, value, constraint) in [
        ("max_tier_bands_per_row", 0, "tier_band_cap"),
        ("max_price_rows_per_plan", 0, "price_row_cap"),
        ("max_custom_interval_days", 0, "interval_days_cap"),
        ("max_custom_interval_months", -1, "interval_months_cap"),
    ] {
        let err = common::exec(
            &conn,
            &format!(
                "INSERT INTO pricing_policy_object (tenant_id, updated_by, {column}) \
                 VALUES ('{tenant}', '{tenant}', {value})"
            ),
        )
        .await
        .expect_err("a non-positive cap must be refused");
        assert!(
            err.to_string()
                .contains(&format!("chk_pricing_policy_object_{constraint}")),
            "the refusal must be this column's own guard: {err}"
        );
    }
}

/// The extension column is `NOT NULL DEFAULT '[]'`, so "no extension" has one
/// spelling: a row written without it reads as the pinned three rather than as a
/// tenant whose required set is unknown.
#[tokio::test]
async fn a_row_written_without_an_extension_reads_as_the_v1_contract() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0xd0_66);
    seed(&provider, tenant, policy_object::ActiveModel::default()).await;

    let policy = repo
        .authoring_policy(&AccessScope::for_tenant(tenant), tenant)
        .await
        .expect("read the authoring policy");
    assert!(policy.additional_required_descriptors().is_empty());
    assert_eq!(
        findings(&policy, &draft(30, BTreeMap::new()), DESCRIPTOR_INCOMPLETE),
        0
    );
    assert_eq!(
        policy.max_price_rows_per_plan(),
        LimitsConfig::default().max_price_rows_per_plan,
        "an all-null cap set is still the ratified default"
    );
}
