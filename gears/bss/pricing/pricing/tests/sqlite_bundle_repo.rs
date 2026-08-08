//! `BundleRepo` — creating a bundle on a plan, and replacing an open draft
//! revision's whole composition under that revision's entity tag.
//!
//! Where `tests/sqlite_bundle_store.rs` drives raw SQL past every repository to
//! prove the **schema**, this suite drives the repository, because what it proves
//! is the repository's: which refusal a caller is told, what the store holds
//! afterwards, and that the compare-and-swap is in SQL rather than in a read.
//!
//! **The composition carries no entity tag of its own.** It rides the plan
//! revision's, exactly as `PlanShapeRepo`'s child sets do and for that module's
//! reason: if editing the component set left the revision's tag alone, two
//! authors editing one draft would both satisfy `If-Match` and silently
//! interleave — the lost update `fr-concurrent-edit` exists to refuse.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::bundle::{
    Absorber, InvoiceItemization, Party, PartyShare, PriceBasis, RevShareGroup,
};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    BundleComponentDraft, BundleRepo, CompositionDraft, NewBundle, NewPlanDraft, PlanRepo,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_10);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 6, hour, 0, 0)
        .single()
        .expect("instant")
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: Uuid::from_u128(0xac_11),
        recorded_at: at(10),
        correlation_id: TEST_CORRELATION,
    }
}

async fn harness() -> (PlanRepo, BundleRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (
        PlanRepo::new(provider.clone()),
        BundleRepo::new(provider.clone()),
        provider,
    )
}

fn new_draft(plan_id: PlanId, tenant_id: Uuid) -> NewPlanDraft {
    NewPlanDraft {
        plan_id,
        tenant_id,
        created_by: Uuid::from_u128(0xac_11),
        created_at_utc: at(9),
        sku_id: Some(Uuid::from_u128(0x5_c1)),
        plan_tier: Some("gold".to_owned()),
        billing_cycle: None,
        frequency: None,
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
        cloned_from: None,
        correlation_id: TEST_CORRELATION,
    }
}

const VENDOR: Uuid = Uuid::from_u128(0x_5e_11);

fn component(id: u128) -> BundleComponentDraft {
    BundleComponentDraft {
        component_plan_id: Uuid::from_u128(id),
        included_sku_id: Uuid::from_u128(id + 0x1000),
        min_qty: None,
        max_qty: None,
    }
}

fn group(shares: &[(&str, i32)], cut: i32) -> RevShareGroup {
    RevShareGroup {
        vendor_sku_id: VENDOR,
        platform_cut_bp: cut,
        residual_absorber: Absorber::Platform,
        parties: shares
            .iter()
            .map(|(n, bp)| PartyShare {
                party: Party::new(n).expect("party"),
                share_bp: *bp,
            })
            .collect(),
    }
}

/// A plan with an open draft revision 0, and a bundle on it.
async fn seeded(
    plans: &PlanRepo,
    bundles: &BundleRepo,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
) -> Uuid {
    plans
        .create_draft(scope, new_draft(plan_id, tenant))
        .await
        .expect("create the plan draft");
    bundles
        .create(
            scope,
            NewBundle {
                bundle_id: Uuid::from_u128(0xb0_1d),
                tenant_id: tenant,
                plan_id,
                price_basis: PriceBasis::SumOfParts,
                invoice_itemization: InvoiceItemization::Aggregate,
            },
            stamp(),
        )
        .await
        .expect("create the bundle")
}

// ---------------------------------------------------------------------------
// Creating the bundle row.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bundle_is_created_on_its_plan_and_read_back() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    let bundle_id = seeded(&plans, &bundles, &scope, tenant, plan_id).await;

    let found = bundles
        .find_by_plan(&scope, tenant, plan_id)
        .await
        .expect("read")
        .expect("the bundle must be there");
    assert_eq!(found.bundle_id, bundle_id);
    assert_eq!(found.price_basis, PriceBasis::SumOfParts);
    assert_eq!(found.invoice_itemization, InvoiceItemization::Aggregate);
}

/// One bundle per plan — `uq_pricing_bundle_plan`, surfaced as a typed refusal
/// rather than as a driver error.
#[tokio::test]
async fn a_second_bundle_on_one_plan_is_refused() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;

    let err = bundles
        .create(
            &scope,
            NewBundle {
                bundle_id: Uuid::from_u128(0xb0_2d),
                tenant_id: tenant,
                plan_id,
                price_basis: PriceBasis::OwnPrice,
                invoice_itemization: InvoiceItemization::Itemize,
            },
            stamp(),
        )
        .await
        .expect_err("one bundle per plan");

    assert!(
        matches!(err, RepoError::DuplicateBundleOnPlan { .. }),
        "unexpected: {err:?}"
    );
}

/// A foreign tenant's bundle is invisible, which is `SecureORM` doing its job
/// through this repository rather than around it.
#[tokio::test]
async fn a_foreign_tenants_bundle_is_invisible() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;

    let stranger = AccessScope::for_tenant(Uuid::from_u128(0x7e_22));
    let found = bundles
        .find_by_plan(&stranger, Uuid::from_u128(0x7e_22), plan_id)
        .await
        .expect("read");
    assert!(found.is_none(), "a foreign tenant must see nothing");
}

// ---------------------------------------------------------------------------
// Replacing the composition under the revision's tag.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_composition_is_written_whole_and_read_back() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;

    let revision = bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            CompositionDraft {
                components: vec![component(1), component(2)],
                rev_share_groups: vec![group(&[("a", 4500), ("b", 4500)], 1000)],
            },
            stamp(),
        )
        .await
        .expect("write the composition");

    // The revision's tag advanced: the composition rides it (see the module doc).
    assert_eq!(revision.row_version, RowVersion::new(1));

    let stored = bundles
        .load_composition(&scope, tenant, plan_id, 0)
        .await
        .expect("read the composition");
    assert_eq!(stored.components.len(), 2);
    assert_eq!(stored.rev_share_groups.len(), 1);
    assert_eq!(stored.rev_share_groups[0].parties.len(), 2);
}

/// Wholesale replacement, never a merge: a second write with one component
/// leaves **one**, not three.
///
/// The reason is `PlanShapeRepo`'s for phases, applied here: the composition is
/// judged as a set — "every referenced component", the per-market coverage walk,
/// "sum to 100% per vendor SKU" — and a partial update has no set the rules
/// could evaluate.
#[tokio::test]
async fn a_second_write_replaces_the_composition_rather_than_merging_it() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;

    bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            CompositionDraft {
                components: vec![component(1), component(2)],
                rev_share_groups: vec![group(&[("a", 9000)], 1000)],
            },
            stamp(),
        )
        .await
        .expect("first write");
    bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(1),
            CompositionDraft {
                components: vec![component(3)],
                rev_share_groups: Vec::new(),
            },
            stamp(),
        )
        .await
        .expect("second write");

    let stored = bundles
        .load_composition(&scope, tenant, plan_id, 0)
        .await
        .expect("read");
    assert_eq!(stored.components.len(), 1, "replaced, not merged");
    assert_eq!(stored.components[0].component_plan_id, Uuid::from_u128(3));
    assert!(stored.rev_share_groups.is_empty(), "the groups went too");
}

/// The compare-and-swap is the guard: a stale tag is refused and nothing is
/// written.
#[tokio::test]
async fn a_stale_row_version_is_refused_and_writes_nothing() {
    let (plans, bundles, _) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;
    bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            CompositionDraft {
                components: vec![component(1)],
                rev_share_groups: Vec::new(),
            },
            stamp(),
        )
        .await
        .expect("first write");

    let err = bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            // The version the first write consumed.
            RowVersion::new(0),
            CompositionDraft {
                components: vec![component(9)],
                rev_share_groups: Vec::new(),
            },
            stamp(),
        )
        .await
        .expect_err("a stale tag must be refused");
    assert!(
        matches!(err, RepoError::StaleRowVersion { .. }),
        "unexpected: {err:?}"
    );

    let stored = bundles
        .load_composition(&scope, tenant, plan_id, 0)
        .await
        .expect("read");
    assert_eq!(
        stored.components[0].component_plan_id,
        Uuid::from_u128(1),
        "the refused write must have landed nothing"
    );
}

/// A published revision's composition is frozen (D-92). The repository refuses
/// before the trigger does, so the caller is told which conjunct failed.
#[tokio::test]
async fn a_published_revisions_composition_cannot_be_replaced() {
    let (plans, bundles, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));
    seeded(&plans, &bundles, &scope, tenant, plan_id).await;
    flip_published(&provider, &scope, plan_id).await;

    let err = bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            CompositionDraft {
                components: vec![component(1)],
                rev_share_groups: Vec::new(),
            },
            stamp(),
        )
        .await
        .expect_err("a published revision is frozen");

    assert!(
        matches!(
            err,
            RepoError::NotDraft { .. } | RepoError::StaleRowVersion { .. }
        ),
        "unexpected: {err:?}"
    );
}

/// Flip revision 0 to `published` past the repository — the state a typed path
/// reaches only through the publish commit.
async fn flip_published(provider: &DBProvider<DbError>, scope: &AccessScope, plan_id: PlanId) {
    use bss_pricing::infra::storage::entity::plan;
    use sea_orm::sea_query::Expr;
    use toolkit_db::secure::SecureUpdateExt;

    let conn = provider.conn().expect("conn");
    plan::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            plan::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::Revision.eq(0_i64)),
        )
        .exec(&conn)
        .await
        .expect("flip");
}
