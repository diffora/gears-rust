//! What a clone copies, what it remaps and what it deliberately leaves behind
//! (`design/12-operator-efficiency.md` §3 `algo-clone`, `inst-cl-*`; D-19,
//! D-264).
//!
//! Driven through the real repositories rather than over a hand-built world,
//! because every claim here is about what the *store* ends up holding — and this
//! program has spent a day on rules whose operand nothing populated. A copy
//! asserted against a fixture the test itself assembled would prove nothing about
//! the copier.
//!
//! # The phase remap is what this suite is mostly for
//!
//! Three sites reference a `phase_id`, and the third is the one the 2026-08-01
//! review found missing (C-7): the chain, the price rows' scope keys, and the
//! keys of the D-41 `entitlement_grants.perPhase` map. A clone that remapped the
//! first two and not the third publishes a grant set pointing at phases that
//! exist only in the source, and fails `GRANT_SET_PHASE_UNKNOWN` on its first
//! publish — which is a refusal the *operator* has no way to act on, the dangling
//! ids being invisible to them.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::contracts::{
    EntitlementGrants, GrantSet, PlanChangeContract, UsageCounterOnPlanChange,
};
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan::PlanShapePatch;
use bss_pricing::domain::plan_shape::{PhaseKind, PlanPhase};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::clone::{CloneNotice, PlanCloner};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    BundleRepo, NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PriceRepo, plan_repo,
    plan_shape_repo, price_repo,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use std::collections::BTreeMap;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const ACTOR: Uuid = Uuid::from_u128(0xac_10);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_11);
const SOURCE_COMPOSITE: Uuid = Uuid::from_u128(0xc0_f1);

fn source_plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x50_c1))
}
fn target_plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x7a_69))
}
fn trial_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_51))
}
fn terminal_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_5e))
}
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 8, hour, 0, 0).unwrap()
}
fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(10),
        correlation_id: CORRELATION,
    }
}

struct Harness {
    provider: DBProvider<DbError>,
    bundles: BundleRepo,
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    cloner: PlanCloner,
    scope: AccessScope,
}

async fn harness() -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let plans = PlanRepo::new(provider.clone());
    let shapes = PlanShapeRepo::new(provider.clone());
    let prices = PriceRepo::new(provider.clone());
    let cloner = PlanCloner::new(
        provider.clone(),
        plans.clone(),
        shapes.clone(),
        prices.clone(),
        BundleRepo::new(provider.clone()),
    );
    Harness {
        provider: provider.clone(),
        bundles: BundleRepo::new(provider),
        plans,
        shapes,
        prices,
        cloner,
        scope: AccessScope::for_tenant(TENANT),
    }
}

fn key_on(plan: PlanId, phase: PhaseId, eligibility: PriceEligibility, cohort: Cohort) -> ScopeKey {
    key_in(plan, phase, eligibility, cohort, "eu")
}

fn key_in(
    plan: PlanId,
    phase: PhaseId,
    eligibility: PriceEligibility,
    cohort: Cohort,
    region: &str,
) -> ScopeKey {
    ScopeKey::new(
        plan,
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        phase,
        eligibility,
        ChargeKind::Recurring,
        cohort,
    )
    .expect("the class pairs with the cohort")
}

fn flat_row() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

/// A published source plan: two phases, a grant set keyed on the trial phase, and
/// one published price row on each phase.
async fn seed_source(h: &Harness) {
    let created = h
        .plans
        .create_draft(
            &h.scope,
            NewPlanDraft {
                plan_id: source_plan(),
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: at(10),
                sku_id: Some(Uuid::from_u128(0x5_c1)),
                plan_tier: Some("gold".to_owned()),
                billing_cycle: Some(bss_pricing::domain::plan_shape::BillingCycle::Recurring),
                frequency: Some(bss_pricing::domain::plan_shape::Frequency::Monthly),
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                invoice_grouping_key: Some("group/source".to_owned()),
                available_from: None,
                available_to: None,
                cloned_from: None,
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("create the source draft");

    let after_phases = h
        .shapes
        .replace_phases(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            created.row_version,
            vec![
                PlanPhase {
                    phase_id: trial_phase(),
                    kind: PhaseKind::Trial,
                    ordinal: 0,
                    converts_to_phase_id: Some(terminal_phase()),
                    phase_duration_days: Some(14),
                    display_trial_days: Some(14),
                },
                PlanPhase {
                    phase_id: terminal_phase(),
                    kind: PhaseKind::Evergreen,
                    ordinal: 1,
                    converts_to_phase_id: None,
                    phase_duration_days: None,
                    display_trial_days: None,
                },
            ],
            stamp(),
        )
        .await
        .expect("attach the chain");

    let after_descriptors = h
        .shapes
        .set_descriptor_set(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            after_phases.row_version,
            bss_pricing::domain::plan_shape::DescriptorSet {
                invoice_line_template: Some("{plan}".to_owned()),
                gl_code: Some("4000".to_owned()),
                itemization_rule: Some("per_charge".to_owned()),
                additional: BTreeMap::new(),
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");

    let after_composites = h
        .shapes
        .replace_composites(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            after_descriptors.row_version,
            vec![bss_pricing::domain::plan_shape::CompositeMeter {
                composite_id: SOURCE_COMPOSITE,
                output_unit: "vm-hour".to_owned(),
                constituent_units: vec!["vcpu-hour".to_owned(), "ram-gb-hour".to_owned()],
                formula: serde_json::json!({ "op": "weighted_sum" }),
            }],
            stamp(),
        )
        .await
        .expect("attach the composite");

    // A per-phase grant set keyed on the trial phase — C-7's subject.
    h.plans
        .update_draft(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            after_composites.row_version,
            PlanShapePatch {
                change_contract: Some(PlanChangeContract {
                    allowed_change_targets: Some(vec![Uuid::from_u128(0x9_1a9)]),
                    comparability_rank: Some(10),
                    usage_counter_on_plan_change: UsageCounterOnPlanChange::Carry,
                }),
                entitlement_grants: Some(EntitlementGrants {
                    plan_tier_ref: None,
                    plan_level: GrantSet {
                        feature_flags: BTreeMap::from([("bss.pricing/api".to_owned(), true)]),
                        quotas: BTreeMap::new(),
                    },
                    per_phase: BTreeMap::from([(
                        trial_phase().get(),
                        GrantSet {
                            feature_flags: BTreeMap::new(),
                            quotas: BTreeMap::from([("seats".to_owned(), 20)]),
                        },
                    )]),
                }),
                ..PlanShapePatch::default()
            },
            stamp(),
        )
        .await
        .expect("author the grant set");

    for (id, phase, eligibility) in [
        (
            Uuid::from_u128(0xb_0001),
            trial_phase(),
            PriceEligibility::AllSubscriptions,
        ),
        (
            Uuid::from_u128(0xb_0002),
            terminal_phase(),
            PriceEligibility::AllSubscriptions,
        ),
        // **The row the reset is actually about**, and it sits in its own market.
        // Every other seeded row already carries `all_subscriptions`, so without
        // this one "resets to all_subscriptions" asserts a value nothing had to
        // move — a probe caught exactly that, staying green with the reset
        // removed. The separate region is not decoration: reset onto the *same*
        // market it would collapse onto the `all_subscriptions` row's canonical
        // key, which is the collision `inst-cl-resets` names for grandfathered
        // rows and does not name for this class. See D-265.
        (
            Uuid::from_u128(0xb_0004),
            trial_phase(),
            PriceEligibility::NewSubscriptionsOnly,
        ),
    ] {
        h.prices
            .create_draft(
                &h.scope,
                TENANT,
                NewPriceDraft {
                    price_id: id,
                    scope_key: key_in(
                        source_plan(),
                        phase,
                        eligibility,
                        Cohort::None,
                        if eligibility == PriceEligibility::NewSubscriptionsOnly {
                            "us"
                        } else {
                            "eu"
                        },
                    ),
                    content: flat_row(),
                    created_by: ACTOR,
                    created_at_utc: at(10),
                    correlation_id: CORRELATION,
                },
            )
            .await
            .expect("author a source row");
        common::publish_row_directly(&h.provider, &h.scope, id).await;
    }
    common::publish_plan_directly(&h.provider, &h.scope, source_plan(), created.revision).await;
}

/// **Every `phase_id` reference moves with the phases** — the chain, the price
/// rows' scope keys, and the `perPhase` grant map's keys.
///
/// The third is C-7's finding and the one this case exists for: a clone that
/// remapped the first two and not the third fails its own first publish with
/// `GRANT_SET_PHASE_UNKNOWN`, naming ids the operator cannot see.
#[tokio::test]
async fn every_phase_reference_is_remapped_including_the_grant_map() {
    let h = harness().await;
    seed_source(&h).await;

    let receipt = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");
    assert_eq!(receipt.phases_copied, 2);
    assert_eq!(receipt.prices_copied, 3);

    let conn = h.provider.conn().expect("conn");
    let clone_phases = plan_shape_repo::load_phase_set(&conn, &h.scope, TENANT, target_plan(), 0)
        .await
        .expect("read the clone's phases");
    let new_ids: Vec<PhaseId> = clone_phases.iter().map(|p| p.phase_id).collect();
    assert_eq!(new_ids.len(), 2);
    for id in &new_ids {
        assert_ne!(*id, trial_phase(), "the clone mints its own phase ids");
        assert_ne!(*id, terminal_phase(), "the clone mints its own phase ids");
    }

    // The chain points inside the clone, not back at the source.
    let converts: Vec<PhaseId> = clone_phases
        .iter()
        .filter_map(|p| p.converts_to_phase_id)
        .collect();
    assert_eq!(converts.len(), 1);
    assert!(
        new_ids.contains(&converts[0]),
        "the conversion target must be one of the clone's own phases, got {:?}",
        converts[0]
    );

    // Every copied price row sits on a phase of the clone.
    let rows = price_repo::load_for_plan(
        &conn,
        &h.scope,
        TENANT,
        target_plan(),
        &[LifecycleState::Draft],
    )
    .await
    .expect("read the clone's rows");
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert!(
            new_ids.contains(&row.scope_key.phase()),
            "a copied row still names a source phase: {:?}",
            row.scope_key.phase()
        );
    }

    // **C-7.** The grant map's keys moved too, and to a phase that exists here.
    let revision = plan_repo::load_open_draft(&conn, &h.scope, TENANT, target_plan())
        .await
        .expect("read the clone's draft")
        .expect("there is one");
    let keys: Vec<Uuid> = revision
        .entitlement_grants
        .per_phase
        .keys()
        .copied()
        .collect();
    assert_eq!(keys.len(), 1, "the per-phase entry is copied, not dropped");
    assert!(
        new_ids.iter().any(|id| id.get() == keys[0]),
        "the perPhase key still names a source phase, so the clone's first \
         publish would fail GRANT_SET_PHASE_UNKNOWN on an id the operator cannot \
         see: {:?}",
        keys[0]
    );
    assert_eq!(
        revision.entitlement_grants.plan_level.feature_flags.len(),
        1,
        "and the plan-level set rides along unchanged"
    );
}

/// The clone is lineage-stamped, in `draft`, and carries the source's config.
#[tokio::test]
async fn the_clone_is_a_draft_that_names_its_source() {
    let h = harness().await;
    seed_source(&h).await;
    h.cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");

    let conn = h.provider.conn().expect("conn");
    let revision = plan_repo::load_open_draft(&conn, &h.scope, TENANT, target_plan())
        .await
        .expect("read it")
        .expect("there is one");
    assert_eq!(revision.cloned_from, Some(source_plan()));
    assert_eq!(revision.lifecycle_state, LifecycleState::Draft);
    assert_eq!(revision.revision, 0, "a clone starts a fresh plan at 0");
    assert_eq!(revision.plan_tier.as_deref(), Some("gold"));
    assert_eq!(
        revision.invoice_grouping_key.as_deref(),
        Some("group/source"),
        "authored configuration comes across"
    );
}

/// **`inst-cl-resets` (O1): eligibility is re-decided, and grandfathered rows are
/// not copied at all.**
///
/// The two are one rule read from both ends. A grandfathered row copied *with*
/// its eligibility reset would land on the same canonical scope key as the
/// `all_subscriptions` row that supersedes it, and the clone's first publish
/// would fail on the duplicate — so the row is left behind and the operator is
/// told, rather than the clone being quietly unpublishable.
#[tokio::test]
async fn eligibility_resets_and_grandfathered_rows_stay_behind() {
    let h = harness().await;
    seed_source(&h).await;

    // A grandfathered generation on the terminal phase's key.
    let grandfathered = Uuid::from_u128(0xb_0003);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: grandfathered,
                scope_key: key_on(
                    source_plan(),
                    terminal_phase(),
                    PriceEligibility::ExistingGrandfathered,
                    Cohort::Generation(at(9)),
                ),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the grandfathered row");
    common::publish_row_directly(&h.provider, &h.scope, grandfathered).await;

    let receipt = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");

    assert_eq!(
        receipt.prices_copied, 3,
        "the three ordinary rows copy and the grandfathered one does not"
    );
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::GrandfatheredRowsNotCopied { rows: 1 }),
        "and the operator is told which rows stayed behind: {:?}",
        receipt.notices
    );

    let conn = h.provider.conn().expect("conn");
    let rows = price_repo::load_for_plan(
        &conn,
        &h.scope,
        TENANT,
        target_plan(),
        &[LifecycleState::Draft],
    )
    .await
    .expect("read the clone's rows");
    for row in &rows {
        assert_eq!(
            row.scope_key.price_eligibility(),
            PriceEligibility::AllSubscriptions,
            "eligibility must be re-decided, so every copied row resets to \
             all_subscriptions"
        );
        assert!(
            row.scope_key.cohort().is_none(),
            "and the cohort follows it -- the two are one fact"
        );
        assert!(
            row.content().grandfather_until.is_none(),
            "grandfatherUntil is the source's tombstone and says nothing here"
        );
        assert!(
            row.content().supersedes_price_id.is_none(),
            "a clone's first row supersedes nothing"
        );
    }
}

/// **`inst-cl-windows`: schedules are never cloned, and the operator is told.**
///
/// The clone's billable rows have no coverage on arrival, so its publish is
/// blocked until fresh windows are scheduled. That is expected rather than a
/// fault, which is why it is a notice on the receipt and not a refusal.
#[tokio::test]
async fn no_window_is_cloned_and_the_receipt_says_so() {
    let h = harness().await;
    seed_source(&h).await;
    let conn = h.provider.conn().expect("conn");
    common::schedule_coverage_window(&conn, &h.scope, TENANT, Uuid::from_u128(0xb_0001), stamp())
        .await;

    let receipt = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::NoCoverageScheduled { rows: 3 }),
        "the receipt must say the clone has no coverage: {:?}",
        receipt.notices
    );

    let windows = bss_pricing::infra::storage::repo::window_repo::list_for_plan(
        &conn,
        &h.scope,
        TENANT,
        target_plan(),
    )
    .await
    .expect("read the clone's windows");
    assert!(
        windows.is_empty(),
        "PriceWindow schedules are Slice 7 runtime state and are never cloned, \
         got {windows:?}"
    );
}

/// A plan with no current revision is not clonable, and the refusal names it.
#[tokio::test]
async fn a_plan_with_nothing_published_cannot_be_cloned() {
    let h = harness().await;
    let err = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect_err("a plan that has never published has no current revision");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("clonable plan"),
        "the refusal must name what was not found, got: {rendered}"
    );
}

/// **The plan-change contract and the rest of the copy set come across** — the
/// case the group owed and did not have.
///
/// `NewPlanDraft` has no field for `change_contract`, so the clone's first draft
/// silently dropped the plan's `allowedChangeTargets`, its `comparabilityRank`
/// and D-113's carry flag — and **nothing refused it**: with no edges, K4 asks
/// for no rank, so the clone published clean and wrong. `open_revision` had
/// already written the comment explaining that an edge list which resets itself
/// is a silent drop out of self-service change; this path did not read it.
///
/// The descriptor set and the composite meter are here for the same reason —
/// §8's copy set names them and the suite never seeded either, so
/// `composites_copied` had never been non-zero and the `composite_id` re-mint had
/// never run.
#[tokio::test]
async fn the_whole_copy_set_comes_across_contract_descriptors_and_composites() {
    let h = harness().await;
    seed_source(&h).await;
    let receipt = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");
    assert_eq!(receipt.composites_copied, 1);

    let conn = h.provider.conn().expect("conn");
    let revision = plan_repo::load_open_draft(&conn, &h.scope, TENANT, target_plan())
        .await
        .expect("read the clone")
        .expect("there is one");

    // The plan-change contract, all three members.
    assert_eq!(
        revision.change_contract.allowed_change_targets,
        Some(vec![Uuid::from_u128(0x9_1a9)]),
        "an edge list that reset itself here drops the clone out of self-service \
         change, and nothing downstream would refuse it"
    );
    assert_eq!(revision.change_contract.comparability_rank, Some(10));
    assert_eq!(
        revision.change_contract.usage_counter_on_plan_change,
        UsageCounterOnPlanChange::Carry,
        "D-113's flag decides whether a subscriber's usage counter survives a \
         plan change; its default is the opposite of this value"
    );

    // The descriptor set.
    let descriptors = plan_shape_repo::load_descriptor(&conn, &h.scope, TENANT, target_plan(), 0)
        .await
        .expect("read the descriptor set")
        .expect("it came across");
    assert_eq!(descriptors.gl_code.as_deref(), Some("4000"));

    // The composite, under a **new** id: `composite_id` is stable across
    // revisions of one plan (D-106), not across plans.
    let composites = plan_shape_repo::load_composite_set(&conn, &h.scope, TENANT, target_plan(), 0)
        .await
        .expect("read the composites");
    assert_eq!(composites.len(), 1);
    assert_ne!(
        composites[0].composite_id, SOURCE_COMPOSITE,
        "the clone mints its own composite id"
    );
    assert_eq!(composites[0].output_unit, "vm-hour");
    assert_eq!(composites[0].constituent_units.len(), 2);
}

/// The source is a bundle and its composition does not come across, so the
/// receipt says so rather than handing back a plan that silently is not one.
///
/// §3's copy set predates Slice 8 and names none of the bundle tables, while
/// `plan_repo::open_revision` copies them as the plan's child tables — the two
/// paths that reproduce a plan disagree, and D-266 records that rather than this
/// path picking a side.
#[tokio::test]
async fn a_bundle_source_is_reported_rather_than_silently_flattened() {
    let h = harness().await;
    seed_source(&h).await;
    h.bundles
        .create(
            &h.scope,
            bss_pricing::infra::storage::repo::NewBundle {
                bundle_id: Uuid::from_u128(0xb_11d),
                tenant_id: TENANT,
                plan_id: source_plan(),
                price_basis: bss_pricing::domain::bundle::PriceBasis::SumOfParts,
                invoice_itemization: bss_pricing::domain::bundle::InvoiceItemization::Itemize,
            },
            stamp(),
        )
        .await
        .expect("make the source a bundle");

    let receipt = h
        .cloner
        .clone_plan(
            &h.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::BundleCompositionNotCopied),
        "cloning a bundle must say the composition stayed behind: {:?}",
        receipt.notices
    );

    // And an ordinary plan raises no such notice, so the assertion above is
    // about the source being a bundle rather than about a constant.
    let h2 = harness().await;
    seed_source(&h2).await;
    let plain = h2
        .cloner
        .clone_plan(
            &h2.scope,
            TENANT,
            source_plan(),
            target_plan(),
            at(11),
            stamp(),
        )
        .await
        .expect("the clone runs");
    assert!(
        !plain
            .notices
            .contains(&CloneNotice::BundleCompositionNotCopied),
        "an ordinary plan is not a bundle: {:?}",
        plain.notices
    );
}
