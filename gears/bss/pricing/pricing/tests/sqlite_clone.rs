//! What a clone copies, what it remaps and what it deliberately leaves behind
//! (`design/12-operator-efficiency.md` §3 `algo-clone`, `inst-cl-*`; D-19,
//! D-264, D-268, D-269).
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
//!
//! # What is asserted about the resets, and what is not
//!
//! Since D-268 both cutover-made eligibility classes are excluded, so **every
//! row that reaches the reset already carries `all_subscriptions` and
//! `cohort = none`**. Those two assertions are therefore fences rather than
//! evidence — true of a value nothing had to move — and are marked as such where
//! they stand, beside the pair D-266 already demoted. What the suite proves
//! instead is the **exclusion**: the clone holds no row on the market only a
//! `new_subscriptions_only` row occupied, the receipt names each class it left
//! behind, and a source holding both classes on **one** market clones without
//! the collapse `inst-cl-resets` names only for grandfathered rows.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::bundle::{
    Absorber, InvoiceItemization, Party, PartyShare, PriceBasis, RevShareGroup,
};
use bss_pricing::domain::contracts::{
    EntitlementGrants, GrantSet, PlanChangeContract, UsageCounterOnPlanChange,
};
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan::PlanShapePatch;
use bss_pricing::domain::plan_shape::{AddonRule, PhaseKind, PlanPhase};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::clone::{CloneNotice, CloneReceipt, clone_plan_on};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    BundleComponentDraft, BundleRepo, CompositionDraft, NewBundle, NewPlanDraft, NewPriceDraft,
    PlanRepo, PlanShapeRepo, PriceRepo, bundle_repo, plan_repo, plan_shape_repo, price_repo,
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
const SOURCE_BUNDLE: Uuid = Uuid::from_u128(0xb_11d);
/// The two plans the source bundle includes. **Other plans**, which the clone
/// did not copy: `component_plan_id` is the one id in the composition that must
/// travel unchanged (D-269).
const COMPONENT_A: Uuid = Uuid::from_u128(0xc0_a1);
const COMPONENT_B: Uuid = Uuid::from_u128(0xc0_b1);
const VENDOR_SKU: Uuid = Uuid::from_u128(0x5e_11);
const ADDON_SKU: Uuid = Uuid::from_u128(0xad_11);

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
    Harness {
        provider: provider.clone(),
        bundles: BundleRepo::new(provider),
        plans,
        shapes,
        prices,
        scope: AccessScope::for_tenant(TENANT),
    }
}

/// The clone, on a transaction this test owns — the only way in (D-275).
///
/// There is no wrapper that opens its own transaction, deliberately: one existed,
/// and a method holding a `DBProvider` is silently nested when called from inside
/// another transaction, because `Db::in_transaction` does not consult the
/// task-local `IN_TX`. So every case here composes the clone the way the route
/// does, which also means a case cannot pass over a path production never takes.
async fn clone_it(h: &Harness) -> Result<CloneReceipt, DomainError> {
    let scope = h.scope.clone();
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<CloneReceipt, DomainError, _>(move |txn| {
            Box::pin(async move {
                Box::pin(clone_plan_on(
                    txn,
                    &scope,
                    TENANT,
                    source_plan(),
                    target_plan(),
                    at(11),
                    stamp(),
                ))
                .await
            })
        })
        .await;
    outcome.map_err(|err| {
        err.into_domain(|infra| DomainError::Internal(format!("bss-pricing: clone: {infra}")))
    })
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

/// The source bundle's composition: two components on two **other** plans, one
/// of them qty-bounded, and one rev-share group whose residual is absorbed by a
/// named party rather than by the platform default.
///
/// Every value here is off the default: a copier that wrote a fresh
/// [`CompositionDraft`] instead of the source's would differ on all of them.
fn source_composition() -> CompositionDraft {
    CompositionDraft {
        components: vec![
            BundleComponentDraft {
                component_plan_id: COMPONENT_A,
                included_sku_id: Uuid::from_u128(0x_5c_0a),
                min_qty: Some(1),
                max_qty: Some(4),
            },
            BundleComponentDraft {
                component_plan_id: COMPONENT_B,
                included_sku_id: Uuid::from_u128(0x_5c_0b),
                min_qty: None,
                max_qty: None,
            },
        ],
        rev_share_groups: vec![RevShareGroup {
            vendor_sku_id: VENDOR_SKU,
            platform_cut_bp: 1_500,
            residual_absorber: Absorber::Party(Party::new("vendor").expect("a party name")),
            parties: vec![
                PartyShare {
                    party: Party::new("reseller").expect("a party name"),
                    share_bp: 2_500,
                },
                PartyShare {
                    party: Party::new("vendor").expect("a party name"),
                    share_bp: 6_000,
                },
            ],
        }],
    }
}

/// A published source plan: two phases, a grant set keyed on the trial phase, and
/// one published price row on each phase.
async fn seed_source(h: &Harness) {
    seed(h, None).await;
}

/// The same source, made a bundle carrying [`source_composition`].
///
/// The composition is written while the source revision is still a **draft**:
/// all three composition tables refuse an INSERT against a published parent, so
/// a bundle seeded after the publish could carry no components at all — which is
/// how a copier asserted against nothing would look green.
async fn seed_bundle_source(h: &Harness) {
    seed(h, Some(source_composition())).await;
}

/// The source's published price rows: two copied, one excluded.
///
/// Its own function because [`seed`] outgrew the line cap when the add-on rule
/// joined it, and this is the seam that leaves both halves readable — the shape
/// above authors *one* revision through a version chain, and this authors rows
/// that hang off it and are published individually.
async fn seed_published_rows(h: &Harness) {
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
        // **The row D-268 leaves behind**, and it sits in its own market so that
        // the exclusion is observable as an *absence* rather than only as a
        // count: nothing else in the seed occupies `us`, so a clone holding a
        // `us` row copied a class it was told not to.
        //
        // It was seeded by D-265 to give the eligibility reset an operand. D-268
        // took that operand away — this class is now excluded, and no row that
        // reaches the reset can carry anything but `all_subscriptions`. The
        // collapse D-265 measured, both classes on one market, is
        // `both_eligibility_classes_on_one_market_survive_the_clone`'s.
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
}

async fn seed(h: &Harness, composition: Option<CompositionDraft>) {
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

    // **The one child class no clone case reached** until the 2026-08-09 review
    // pointed it out: `replace_addon_rules` had no operand here, so the writer
    // whose faithfulness D-274 called measured was measured by nothing. Every
    // field is off its default, so a copier writing a fresh `AddonRule` would
    // differ on all of them.
    let after_rules = h
        .shapes
        .replace_addon_rules(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            after_phases.row_version,
            vec![AddonRule {
                addon_sku_id: ADDON_SKU,
                required: true,
                min_qty: Some(1),
                max_qty: Some(3),
                step_qty: Some(1),
                price_override_ref: None,
                depends_on: Vec::new(),
                conflicts_with: Vec::new(),
            }],
            stamp(),
        )
        .await
        .expect("attach the add-on rule");

    let after_descriptors = h
        .shapes
        .set_descriptor_set(
            &h.scope,
            TENANT,
            source_plan(),
            created.revision,
            after_rules.row_version,
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
    let after_grants = h
        .plans
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

    if let Some(draft) = composition {
        h.bundles
            .create(
                &h.scope,
                NewBundle {
                    bundle_id: SOURCE_BUNDLE,
                    tenant_id: TENANT,
                    plan_id: source_plan(),
                    // Neither value is its enum's first variant, so a clone that
                    // wrote a constant pair would differ on at least one.
                    price_basis: PriceBasis::OwnPrice,
                    invoice_itemization: InvoiceItemization::Aggregate,
                },
                stamp(),
            )
            .await
            .expect("make the source a bundle");
        h.bundles
            .replace_composition(
                &h.scope,
                TENANT,
                source_plan(),
                created.revision,
                after_grants.row_version,
                draft,
                stamp(),
            )
            .await
            .expect("author the source composition");
    }

    seed_published_rows(h).await;
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

    let receipt = clone_it(&h).await.expect("the clone runs");
    assert_eq!(receipt.phases_copied, 2);
    assert_eq!(
        receipt.prices_copied, 2,
        "the two all_subscriptions rows; the new_subscriptions_only row stays \
         behind (D-268)"
    );

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
    assert_eq!(rows.len(), 2);
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
    clone_it(&h).await.expect("the clone runs");

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

/// **Both cutover-made classes stay behind, and the receipt names each** —
/// `inst-cl-resets` for grandfathered rows, D-268 for `new_subscriptions_only`.
///
/// One rule read from both ends. A row of either class copied *with* its
/// eligibility reset lands on the same canonical scope key as the
/// `all_subscriptions` row beside it, and the clone's first publish fails on the
/// duplicate — so the row is left behind and the operator is told, rather than
/// the clone being quietly unpublishable. The second class is meaningless on a
/// clone twice over: every subscription on a new plan is new.
#[tokio::test]
async fn both_cutover_classes_stay_behind_and_the_receipt_names_each() {
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

    let receipt = clone_it(&h).await.expect("the clone runs");

    assert_eq!(
        receipt.prices_copied, 2,
        "the two all_subscriptions rows copy; the grandfathered row and the \
         new_subscriptions_only row do not"
    );
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::GrandfatheredRowsNotCopied { rows: 1 }),
        "and the operator is told which rows stayed behind: {:?}",
        receipt.notices
    );
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::NewSubscriptionsOnlyRowsNotCopied { rows: 1 }),
        "each class is named on its own, so the operator can tell a retained \
         generation from a cutover's going-forward row: {:?}",
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

    // **The exclusion, as an absence.** Only the `new_subscriptions_only` row
    // occupies `us` in the seed, so a clone holding one copied the class it was
    // told to leave. This is the assertion that bites; the loop below no longer
    // does.
    assert!(
        rows.iter()
            .all(|row| row.scope_key.region().as_str() != "us"),
        "the clone holds a row on the market only the new_subscriptions_only \
         row occupied: {:?}",
        rows.iter()
            .map(|row| row.scope_key.region().as_str().to_owned())
            .collect::<Vec<_>>()
    );

    // **Three structural fences, and a fourth assertion that is not one.**
    //
    // The eligibility, the cohort and `grandfatherUntil` are already true of
    // every row that reaches the reset, because the two classes that could carry
    // anything else are excluded above — D-266 demoted the cohort and the
    // grandfather tombstone for that reason, and D-268 does the same to the
    // eligibility. Those three are kept so that a later change admitting either
    // class fails here rather than at a clone's first publish, and are claimed as
    // nothing more.
    //
    // **`supersedes_price_id` is a different thing and the sentence above used to
    // cover it, wrongly.** A published row that superseded a predecessor really
    // does carry one — `price_repo::insert_successor_draft_on` and
    // `infra::supersession` both set it — and those are exactly the published
    // `all_subscriptions` rows the clone copies. So the reset has a live operand,
    // and `a_superseding_row_loses_its_predecessor_link` below is what holds it;
    // filing it as a fence is how a real rule gets deleted by someone tidying.
    for row in &rows {
        assert_eq!(
            row.scope_key.price_eligibility(),
            PriceEligibility::AllSubscriptions
        );
        assert!(row.scope_key.cohort().is_none());
        assert!(row.content().grandfather_until.is_none());
    }
}

/// **A copied row supersedes nothing** — the reset D-266 recorded as having a real
/// operand and no case, given one.
///
/// A published source row that superseded a predecessor carries
/// `supersedesPriceId` pointing into the **source's** chain. Copied unchanged, the
/// clone's first draft row would claim to supersede a row belonging to another
/// plan — a chain the clone was never part of, and one its own first publish has
/// no business continuing.
///
/// The seed goes through the store rather than through the supersession engine:
/// what is under test is that the cloner clears the field, not how it came to be
/// set, and `update_draft` on a draft row is the cheapest way to put a real value
/// there.
#[tokio::test]
async fn a_superseding_row_loses_its_predecessor_link() {
    let h = harness().await;
    seed_source(&h).await;

    // A published row that supersedes a predecessor, authored through the
    // ordinary path: `PriceContent` carries the link, so no raw SQL is needed.
    let predecessor = Uuid::from_u128(0xb_00fe);
    let successor = Uuid::from_u128(0xb_00ff);
    let mut content = flat_row();
    content.supersedes_price_id = Some(predecessor);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: successor,
                scope_key: key_in(
                    source_plan(),
                    terminal_phase(),
                    PriceEligibility::AllSubscriptions,
                    Cohort::None,
                    "de",
                ),
                content,
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author a superseding row");
    common::publish_row_directly(&h.provider, &h.scope, successor).await;

    let conn = h.provider.conn().expect("conn");
    let before = price_repo::load_for_plan(
        &conn,
        &h.scope,
        TENANT,
        source_plan(),
        &[LifecycleState::Published],
    )
    .await
    .expect("read the source");
    assert!(
        before
            .iter()
            .any(|row| row.content().supersedes_price_id == Some(predecessor)),
        "the seed must actually carry a predecessor link, or this case proves \
         nothing about clearing one"
    );

    clone_it(&h).await.expect("the clone runs");

    let rows = price_repo::load_for_plan(
        &conn,
        &h.scope,
        TENANT,
        target_plan(),
        &[LifecycleState::Draft],
    )
    .await
    .expect("read the clone's rows");
    assert!(!rows.is_empty(), "the clone copied rows at all");
    for row in &rows {
        assert!(
            row.content().supersedes_price_id.is_none(),
            "a clone's first row supersedes nothing, and least of all a row of \
             the plan it was copied from"
        );
    }
}

/// **A source holding both eligibility classes on one market clones cleanly** —
/// the collapse `inst-cl-resets` names only for grandfathered rows, closed for
/// the class it did not name (D-268).
///
/// `(EUR, eu, terminal, recurring)` carries an `all_subscriptions` row and a
/// `new_subscriptions_only` row: two distinct keys the published plane admits,
/// which the reset would send onto one. Measured under D-265: seeding exactly
/// this pair reddened most of the suite with a driver refusal on the draft
/// plane's unique index. It is the reason the class is excluded rather than
/// reset, so it is the case that proves the exclusion is load-bearing and not
/// merely tidy.
#[tokio::test]
async fn both_eligibility_classes_on_one_market_survive_the_clone() {
    let h = harness().await;
    seed_source(&h).await;

    let colliding = Uuid::from_u128(0xb_0005);
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: colliding,
                scope_key: key_on(
                    source_plan(),
                    terminal_phase(),
                    PriceEligibility::NewSubscriptionsOnly,
                    Cohort::None,
                ),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the cutover's going-forward row beside the row it closed");
    common::publish_row_directly(&h.provider, &h.scope, colliding).await;

    let receipt = clone_it(&h)
        .await
        .expect("the clone runs rather than colliding with itself");
    assert_eq!(receipt.prices_copied, 2);
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::NewSubscriptionsOnlyRowsNotCopied { rows: 2 }),
        "both rows of the class are left behind and counted: {:?}",
        receipt.notices
    );
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

    let receipt = clone_it(&h).await.expect("the clone runs");
    assert!(
        receipt
            .notices
            .contains(&CloneNotice::NoCoverageScheduled { rows: 2 }),
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

/// **A clone that fails partway leaves nothing behind.**
///
/// The property D-275 exists for, and one no test could state while the steps
/// were repository *methods*: each opened its own transaction, so a plan draft
/// carrying phases, rules and composites committed step by step, and a caller
/// that failed afterwards had no way to take any of it back. (D-274 built the
/// runner-taking forms this composes; it did not make the clone atomic.)
///
/// The failure is the **caller's**, deliberately, because every failure the
/// clone could raise from its own data is unreachable. Of the four scope-key axes
/// the reset changes — plan, phase, eligibility, cohort — the two that could
/// collapse two source rows onto one target key are operand-free by the time a
/// row gets here (D-266, D-268), and the phase remap is injective. So there is no
/// source a valid author can write that makes the copy refuse halfway. This is
/// also the route's own shape: `idempotent::guarded` owns the transaction, and
/// what fails after the clone is the guard's own bookkeeping.
#[tokio::test]
async fn a_clone_its_caller_rolls_back_leaves_no_row_behind() {
    let h = harness().await;
    seed_source(&h).await;

    let scope = h.scope.clone();
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<(), DomainError, _>(move |txn| {
            Box::pin(async move {
                Box::pin(clone_plan_on(
                    txn,
                    &scope,
                    TENANT,
                    source_plan(),
                    target_plan(),
                    at(11),
                    stamp(),
                ))
                .await?;
                Err(DomainError::Internal(
                    "the caller fails after the clone".to_owned(),
                ))
            })
        })
        .await;
    assert!(outcome.is_err(), "the caller's failure stands");

    let conn = h.provider.conn().expect("conn");
    assert!(
        plan_repo::load_open_draft(&conn, &h.scope, TENANT, target_plan())
            .await
            .expect("read the draft")
            .is_none(),
        "the draft plan the clone created must not survive its caller's rollback"
    );
    assert!(
        price_repo::load_for_plan(
            &conn,
            &h.scope,
            TENANT,
            target_plan(),
            &[LifecycleState::Draft],
        )
        .await
        .expect("read the rows")
        .is_empty(),
        "and neither may the price rows it copied"
    );
    assert!(
        bundle_repo::find_by_plan_on(&conn, &h.scope, TENANT, target_plan())
            .await
            .expect("read the bundle")
            .is_none(),
        "nor the bundle identity it minted"
    );
    assert!(
        plan_shape_repo::load_phase_set(&conn, &h.scope, TENANT, target_plan(), 0)
            .await
            .expect("read the phases")
            .is_empty(),
        "nor the phases it remapped"
    );
}

/// A plan with no current revision is not clonable, and the refusal names it.
#[tokio::test]
async fn a_plan_with_nothing_published_cannot_be_cloned() {
    let h = harness().await;
    let err = clone_it(&h)
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
    let receipt = clone_it(&h).await.expect("the clone runs");
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
    // The add-on rule travels whole and is **not** remapped: `AddonRule` carries
    // no phase, so the rule set is a copy rather than a remap — asserted rather
    // than assumed, which is the module doc's own standard for that claim.
    let rules = plan_shape_repo::load_addon_rule_set(&conn, &h.scope, TENANT, target_plan(), 0)
        .await
        .expect("read the clone's add-on rules");
    assert_eq!(rules.len(), 1, "the source's one rule came across");
    assert_eq!(
        rules[0].addon_sku_id, ADDON_SKU,
        "naming the same add-on SKU: it is another SKU, not one of the clone's own children"
    );
    assert!(rules[0].required);
    assert_eq!(rules[0].min_qty, Some(1));
    assert_eq!(rules[0].max_qty, Some(3));
    assert_eq!(rules[0].step_qty, Some(1));

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

/// **A clone of a bundle is a bundle** (D-269): a new `bundle_id`, and the whole
/// composition under it.
///
/// §3's copy set predates Slice 8 and names none of the bundle tables, while
/// `plan_repo::open_revision` copies them as the plan's child tables — a bundle
/// rides its plan's revisions. The two paths that reproduce a plan disagreed,
/// and this one handed back a plan holding the bundle's price rows and none of
/// its composition.
///
/// The identity is re-minted because `pricing_bundle.plan_id` is unique per
/// plan; `component_plan_id` is **not**, because it names other plans — the
/// clone copied one plan, not the catalogue around it.
#[tokio::test]
async fn a_bundle_clone_is_a_bundle_under_its_own_identity() {
    let h = harness().await;
    seed_bundle_source(&h).await;

    clone_it(&h).await.expect("the clone runs");

    let bundle = h
        .bundles
        .find_by_plan(&h.scope, TENANT, target_plan())
        .await
        .expect("read the clone's bundle")
        .expect("a clone of a bundle is a bundle");
    assert_ne!(
        bundle.bundle_id, SOURCE_BUNDLE,
        "the clone mints its own bundle id"
    );
    assert_eq!(
        bundle.price_basis,
        PriceBasis::OwnPrice,
        "the declared basis is authored configuration and comes across"
    );
    assert_eq!(bundle.invoice_itemization, InvoiceItemization::Aggregate);

    let composition = h
        .bundles
        .load_composition(&h.scope, TENANT, target_plan(), 0)
        .await
        .expect("read the clone's composition");
    assert_eq!(
        composition,
        source_composition(),
        "the whole composition rides the clone: components in their authored \
         order with their qty bounds, and the rev-share group with its cut, its \
         named absorber and both typed shares"
    );
    assert_eq!(
        composition.components[0].component_plan_id, COMPONENT_A,
        "component_plan_id names another plan and must never be remapped -- \
         those are different plans, not the clone's phases"
    );
    assert_eq!(composition.components[1].component_plan_id, COMPONENT_B);

    // The source keeps its own bundle, under its own id: the copy is a copy.
    let source_bundle = h
        .bundles
        .find_by_plan(&h.scope, TENANT, source_plan())
        .await
        .expect("read the source's bundle")
        .expect("the source is still a bundle");
    assert_eq!(source_bundle.bundle_id, SOURCE_BUNDLE);

    // And an ordinary plan clones to an ordinary plan, so the assertions above
    // are about the source being a bundle rather than about a constant.
    let h2 = harness().await;
    seed_source(&h2).await;
    clone_it(&h2).await.expect("the clone runs");
    assert!(
        h2.bundles
            .find_by_plan(&h2.scope, TENANT, target_plan())
            .await
            .expect("read the plain clone's bundle")
            .is_none(),
        "an ordinary plan is not a bundle, and its clone must not become one"
    );
}
