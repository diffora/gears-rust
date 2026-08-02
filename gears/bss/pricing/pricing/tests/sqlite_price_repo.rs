//! Draft price rows and their tier bands against a real database.
//!
//! Almost nothing here is a property of a branch in Rust. The compare-and-swap
//! is a conjunction the database evaluates under the row lock; the band set's
//! order is an `ORDER BY` and not an in-memory sort of what the caller happened
//! to hand over; the foreign key decides in which order two DELETEs may run; and
//! the scope-key check has to see rows this process never wrote. A mock would
//! assert that the repository's own `if` fires and would keep asserting it after
//! the predicate that matters had been deleted.
//!
//! Four things get more room than their line count suggests.
//!
//! The **rollback** is driven by a `flat` row carrying bands, which is the only
//! shape that fails *between* the two writes: the row lands, and the band
//! table's kind trigger refuses the statement after it. Every other refusal in
//! this file happens before the first write or after the last, so the whole
//! suite passed with `in_transaction` replaced by sequential statements on a
//! plain connection — the repository's headline claim, "both tables or
//! neither", asserted by nothing.
//!
//! The **band order** is pinned from rows the database physically holds the
//! wrong way round, not from an out-of-order authoring: `create_draft`
//! normalizes before it writes, so a case driven through it would certify an
//! in-memory sort and leave the read side unpinned. `update_draft` does not
//! normalize, so it is what puts descending rows in the table, and the suite
//! reads the table with no `ORDER BY` to prove they are really there.
//!
//! The **occupied-key** cases are driven for a draft, a published and a
//! superseded occupant, because the repository's check answers all three while
//! the two partial `UNIQUE` indexes answer none of them the same way — and the
//! draft index gets a case of its own, driven through the entity rather than the
//! repository, because it is the half that decides a race the check can only
//! read for.
//!
//! The **content write** is exercised by an update that moves every column at
//! once and clears four of them. An update that submitted content identical to
//! what is stored except for the field under test would pass with any other
//! column silently dropped from the UPDATE.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{
    AggregationFunction, AggregationGranularity, BillingGranularity, IncludedAllowance, ModelKind,
    PriceRow, QuantitySource, RolloverPolicy, TierAggregationWindow, TierBand,
    TierQualificationWindow,
};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::storage::entity::{price, price_tier_band};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPriceDraft, PriceRepo};
use bss_pricing::infra::storage::{RepoError, repo_failure};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureInsertExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

/// The repository, plus the provider the seeding helper needs to put a row into
/// a state only the publish unit (G5) will be able to reach.
async fn harness() -> (PriceRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (PriceRepo::new(provider.clone()), provider)
}

fn tenant() -> Uuid {
    Uuid::from_u128(0x7e_11)
}

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_1a4))
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 2, hour, 0, 0).unwrap()
}

fn money(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("a non-negative amount")
}

/// The default key: `all_subscriptions`, `cohort = none`.
fn base_key(charge_kind: ChargeKind) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::AllSubscriptions,
        charge_kind,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none")
}

/// The third eligibility class's key. It carries `cohort = none` like the
/// default class does — the cohort axis discriminates *retained* generations,
/// and this one retains nobody.
fn new_subscriptions_key(charge_kind: ChargeKind) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::NewSubscriptionsOnly,
        charge_kind,
        Cohort::None,
    )
    .expect("new_subscriptions_only pairs with cohort none")
}

/// A grandfathered generation's key.
///
/// The cohort axis cannot move on its own: `cohort != none` **if and only if**
/// `price_eligibility = existing_grandfathered`, enforced by the domain
/// constructor and again by `chk_pricing_price_cohort_eligibility`. So the two
/// axes move together here, which is what a real cutover does.
fn grandfathered_key(charge_kind: ChargeKind, cutover: DateTime<Utc>) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::ExistingGrandfathered,
        charge_kind,
        Cohort::Generation(cutover),
    )
    .expect("existing_grandfathered pairs with a generation")
}

/// The simplest publishable-looking shape: a flat recurring amount.
fn flat_content() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(money(1_000));
    PriceContent {
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

/// A tiered usage row carrying every Slice-3 evaluation-policy field the kind
/// admits, plus the D-45 allowance declaration and the grandfathering horizon —
/// so a mapping that dropped one column fails here rather than at publish.
fn graduated_content() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.bands = vec![
        TierBand::closed(0, 100, money(0)),
        TierBand::closed(100, 1_000, money(25)),
        TierBand::open(1_000, money(10)),
    ];
    row.meter = Some("api_calls".to_owned());
    "region:eu".clone_into(&mut row.dimension_key);
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);
    row.aggregation_function = Some(AggregationFunction::TimeWeighted);
    row.aggregation_granularity = Some(AggregationGranularity::Hour);
    row.max_hold_granules = Some(6);
    row.included_allowance = Some(IncludedAllowance {
        quantity: 50,
        rollover_policy: RolloverPolicy::Carry,
    });
    PriceContent {
        row,
        tax_inclusive: true,
        billing_timing: Some("arrears".to_owned()),
        rounding_policy_ref: Some("half_even".to_owned()),
        grandfather_until: Some(at(23)),
        supersedes_price_id: Some(Uuid::from_u128(0xb_0f)),
    }
}

fn draft(price_id: Uuid, scope_key: ScopeKey, content: PriceContent) -> NewPriceDraft {
    NewPriceDraft {
        price_id,
        scope_key,
        content,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: at(10),
    }
}

/// Move a row's `lifecycle_state` directly.
///
/// The publish unit that owns this flip lands in G5, and the append-only
/// trigger permits both edges used here: it fires only when the row is already
/// past `draft`, and `published -> superseded` is one of the two flips it
/// whitelists.
async fn flip_state(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
    state: LifecycleState,
) {
    let conn = provider.conn().expect("conn");
    let result = price::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(price::Column::LifecycleState, Expr::value(state.as_str()))
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .exec(&conn)
        .await
        .expect("flip the lifecycle state");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

/// The band rows physically present under `price_id`, whatever the repository
/// would say about them.
async fn stored_bands(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) -> Vec<price_tier_band::Model> {
    let conn = provider.conn().expect("conn");
    price_tier_band::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(price_tier_band::Column::PriceId.eq(price_id)))
        .all(&conn)
        .await
        .expect("read the band table directly")
}

#[tokio::test]
async fn a_created_row_and_its_bands_read_back_whole() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_10);
    let key = grandfathered_key(ChargeKind::Usage, at(9));

    let created = repo
        .create_draft(
            &scope,
            tenant(),
            draft(price_id, key.clone(), graduated_content()),
        )
        .await
        .expect("create the draft row");

    // Draft at version 0: the two facts every later assertion is measured
    // against.
    assert_eq!(created.lifecycle_state, LifecycleState::Draft);
    assert_eq!(created.row_version, RowVersion::new(0));

    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("the row just created is there");

    // Field for field. A mapping that dropped a column would still round-trip
    // the identity, and the drop would only surface at publish — or, for
    // `included_allowance`, as a D-129 supersession guard reporting that
    // nothing had changed.
    assert_eq!(read, created);
    assert_eq!(read.scope_key, key);
    assert_eq!(read.row.model_kind, Some(ModelKind::Graduated));
    assert_eq!(read.row.charge_kind, ChargeKind::Usage);
    assert_eq!(read.row.meter.as_deref(), Some("api_calls"));
    assert_eq!(read.row.dimension_key, "region:eu");
    assert_eq!(
        read.row.billing_granularity,
        Some(BillingGranularity::PerHour)
    );
    assert_eq!(
        read.row.tier_aggregation_window,
        Some(TierAggregationWindow::CalendarMonth)
    );
    assert_eq!(
        read.row.tier_qualification_window,
        Some(TierQualificationWindow::TrailingPeriod)
    );
    assert_eq!(
        read.row.aggregation_function,
        Some(AggregationFunction::TimeWeighted)
    );
    assert_eq!(
        read.row.aggregation_granularity,
        Some(AggregationGranularity::Hour)
    );
    assert_eq!(read.row.max_hold_granules, Some(6));
    assert_eq!(
        read.row.included_allowance,
        Some(IncludedAllowance {
            quantity: 50,
            rollover_policy: RolloverPolicy::Carry,
        })
    );
    assert_eq!(read.row.bands, graduated_content().row.bands);
    assert!(read.tax_inclusive);
    assert_eq!(read.billing_timing.as_deref(), Some("arrears"));
    assert_eq!(read.rounding_policy_ref.as_deref(), Some("half_even"));
    assert_eq!(read.grandfather_until, Some(at(23)));
    assert_eq!(read.supersedes_price_id, Some(Uuid::from_u128(0xb_0f)));
    assert_eq!(read.created_by, Uuid::from_u128(0xac_10));
    assert_eq!(read.created_at_utc, at(10));
}

#[tokio::test]
async fn the_band_set_comes_back_in_quantity_order_however_the_rows_were_written() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_11);

    let ascending = vec![
        TierBand::closed(0, 100, money(0)),
        TierBand::closed(100, 1_000, money(25)),
        TierBand::open(1_000, money(10)),
    ];
    let descending = vec![
        TierBand::open(1_000, money(10)),
        TierBand::closed(100, 1_000, money(25)),
        TierBand::closed(0, 100, money(0)),
    ];

    let created = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                price_id,
                grandfathered_key(ChargeKind::Usage, at(9)),
                graduated_content(),
            ),
        )
        .await
        .expect("create");
    assert_eq!(created.row.bands, ascending);

    // `update_draft` does **not** normalize: it deletes the band set and writes
    // the caller's, row by row, in the order given. So this update is what
    // actually puts descending rows into the table, and it is the only way this
    // suite can reach that state — `create_draft` sorts first, so a test driven
    // through it would prove an in-memory sort and leave the read side unpinned.
    let mut content = graduated_content();
    content.row.bands = descending.clone();
    repo.update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect("replace the band set, descending");

    // The physical rows really are the wrong way round: this reads the table
    // with no ORDER BY at all, so it is measuring what the repository has to
    // correct rather than restating what it did.
    let physical: Vec<u64> = stored_bands(&provider, &scope, price_id)
        .await
        .iter()
        .map(|band| u64::try_from(band.from_qty).expect("a non-negative bound"))
        .collect();
    assert_eq!(
        physical,
        vec![1_000, 100, 0],
        "the update must have written the rows in the order it was given, \
         or this test is no longer measuring the read-side guarantee"
    );

    // And a read still answers ascending. The table is keyed
    // `(price_id, from_qty)` and has no ordinal, so authoring order does not
    // survive persistence; `TierBandValidator` judges geometry over the set
    // sorted by `from_qty` for that reason, and a repository that answered in
    // stored order would let a row pass the save-time pre-check and fail the
    // identical re-run inside the publish commit.
    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.bands, ascending);
}

#[tokio::test]
async fn the_per_kind_money_columns_round_trip_on_the_kinds_that_carry_them() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    // `package`: the money lives in the block columns, which
    // `chk_pricing_price_package_fields_kind` permits on this kind alone.
    let mut package = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    package.package_size = Some(1_000);
    package.package_price_minor = Some(money(4_999));
    package.meter = Some("gb_egress".to_owned());
    package.billing_granularity = Some(BillingGranularity::WholeUnit);
    package.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    let package_id = Uuid::from_u128(0xb_20);
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            package_id,
            base_key(ChargeKind::Usage),
            PriceContent {
                row: package,
                tax_inclusive: false,
                billing_timing: None,
                rounding_policy_ref: None,
                grandfather_until: None,
                supersedes_price_id: None,
            },
        ),
    )
    .await
    .expect("create the package row");

    // `per_unit` on a non-usage row: the unit price on the row, and the
    // quantity the subscription cannot supply stated as a fixed one.
    let mut per_unit = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::PerUnit));
    per_unit.amount_minor = Some(money(1_500));
    per_unit.quantity_source = Some(QuantitySource::Manual);
    per_unit.manual_quantity = Some(12);
    let per_unit_id = Uuid::from_u128(0xb_21);
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            per_unit_id,
            base_key(ChargeKind::Recurring),
            PriceContent {
                row: per_unit,
                tax_inclusive: false,
                billing_timing: Some("advance".to_owned()),
                rounding_policy_ref: None,
                grandfather_until: None,
                supersedes_price_id: None,
            },
        ),
    )
    .await
    .expect("create the per-unit row");

    let read = repo
        .find(&scope, tenant(), package_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.package_size, Some(1_000));
    assert_eq!(read.row.package_price_minor, Some(money(4_999)));
    assert_eq!(read.row.amount_minor, None);
    // Both rows were authored tax-exclusive. The rich round-trip above reads
    // `true`, so without this a mapping that answered a constant would survive
    // the suite.
    assert!(!read.tax_inclusive);

    let read = repo
        .find(&scope, tenant(), per_unit_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.amount_minor, Some(money(1_500)));
    assert_eq!(read.row.quantity_source, Some(QuantitySource::Manual));
    assert_eq!(read.row.manual_quantity, Some(12));
    assert!(!read.tax_inclusive);
}

#[tokio::test]
async fn a_key_held_by_a_draft_or_a_published_row_takes_no_second_row() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let first = Uuid::from_u128(0xb_30);
    let second = Uuid::from_u128(0xb_31);
    let key = base_key(ChargeKind::Recurring);

    repo.create_draft(&scope, tenant(), draft(first, key.clone(), flat_content()))
        .await
        .expect("the first row takes the key");

    // The draft case is this repository's alone: `uq_pricing_price_scope_key_current`
    // is partial over `published` and cannot see it. Two drafts on one key is
    // the ambiguity publish would fail on, found a round trip earlier.
    let err = repo
        .create_draft(&scope, tenant(), draft(second, key.clone(), flat_content()))
        .await
        .expect_err("a second draft on one key must be refused");
    let RepoError::DuplicateScopeKey(detail) = err else {
        panic!("an occupied key must refuse with DUPLICATE_SCOPE_KEY");
    };
    assert!(detail.starts_with(&key.to_string()), "got: {detail}");
    assert!(detail.contains("draft"), "got: {detail}");
    assert!(detail.contains(&first.to_string()), "got: {detail}");

    // Publishing the occupant does not free the key either.
    flip_state(&provider, &scope, first, LifecycleState::Published).await;
    let err = repo
        .create_draft(&scope, tenant(), draft(second, key, flat_content()))
        .await
        .expect_err("a draft may not land on a published key");
    let RepoError::DuplicateScopeKey(detail) = err else {
        panic!("an occupied key must refuse with DUPLICATE_SCOPE_KEY");
    };
    assert!(detail.contains("published"), "got: {detail}");
}

#[tokio::test]
async fn a_superseded_row_no_longer_holds_its_key() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let predecessor = Uuid::from_u128(0xb_40);
    let successor = Uuid::from_u128(0xb_41);
    let key = base_key(ChargeKind::Recurring);

    repo.create_draft(
        &scope,
        tenant(),
        draft(predecessor, key.clone(), flat_content()),
    )
    .await
    .expect("create");
    flip_state(&provider, &scope, predecessor, LifecycleState::Published).await;
    flip_state(&provider, &scope, predecessor, LifecycleState::Superseded).await;

    // A superseded row is retained history and is not the **current** row on
    // its key (§4.3). Treating it as an occupant would make a key unusable
    // forever after its first reprice.
    repo.create_draft(&scope, tenant(), draft(successor, key, flat_content()))
        .await
        .expect("a superseded predecessor leaves the key free");
}

#[tokio::test]
async fn a_different_charge_kind_or_cohort_is_a_different_key() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xb_50),
            base_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("the recurring component");

    // §4.1: a hybrid plan legitimately holds a `recurring` **and** a `usage`
    // row on one plan, currency, region and phase. Without `chargeKind` in the
    // key the second would be rejected as a duplicate of the first.
    let mut usage = flat_content();
    usage.row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    usage.row.amount_minor = Some(money(5));
    usage.row.meter = Some("api_calls".to_owned());
    usage.row.billing_granularity = Some(BillingGranularity::WholeUnit);
    repo.create_draft(
        &scope,
        tenant(),
        draft(Uuid::from_u128(0xb_51), base_key(ChargeKind::Usage), usage),
    )
    .await
    .expect("the usage component is a different key");

    // And every cutover mints a **new** generation on its own key, so retained
    // generations never collide with each other or with the successor.
    let mut retained = flat_content();
    retained.grandfather_until = Some(at(23));
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xb_52),
            grandfathered_key(ChargeKind::Recurring, at(9)),
            retained.clone(),
        ),
    )
    .await
    .expect("the first retained generation");
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xb_53),
            grandfathered_key(ChargeKind::Recurring, at(12)),
            retained,
        ),
    )
    .await
    .expect("a second cutover is a second generation, not a duplicate");
}

#[tokio::test]
async fn a_new_subscriptions_only_row_round_trips_and_is_its_own_key() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let new_only = Uuid::from_u128(0xb_58);
    let all = Uuid::from_u128(0xb_59);

    // The third normative class (PRD 1.4 glossary + 6.9, AC #59, S7 W3 /
    // `inst-el-fields`). It existed in the design set and in neither the enum,
    // the CHECK nor the repository's inverse list, so a row carrying it could
    // not be authored — and had one reached the table any read of it would have
    // answered `CorruptRow` forever.
    let created = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                new_only,
                new_subscriptions_key(ChargeKind::Recurring),
                flat_content(),
            ),
        )
        .await
        .expect("a new_subscriptions_only row is authorable");

    let read = repo
        .find(&scope, tenant(), new_only)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read, created);
    assert_eq!(
        read.scope_key.price_eligibility(),
        PriceEligibility::NewSubscriptionsOnly
    );
    assert_eq!(read.scope_key.cohort(), Cohort::None);

    // And it is a **different key** from the `all_subscriptions` row on every
    // other axis it shares, which is what lets both hold a current row at once —
    // the state W3's most-specific-wins rule exists to resolve. An eligibility
    // axis missing from the duplicate check would refuse this second row as a
    // duplicate and make the promo class unauthorable beside its base.
    repo.create_draft(
        &scope,
        tenant(),
        draft(all, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("the all_subscriptions row on the same remaining axes is a second key");

    let both = repo
        .list_for_plan(&scope, tenant(), plan(), &[LifecycleState::Draft])
        .await
        .expect("list drafts");
    assert_eq!(both.len(), 2, "both classes hold a row of their own");
    assert_ne!(both[0].scope_key, both[1].scope_key);
}

#[tokio::test]
async fn a_grandfathering_horizon_off_its_class_is_the_callers_mistake_not_the_stores() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_5a);

    // `grandfather_until` is ordinary draft content and no domain rule reads it,
    // while `chk_pricing_price_grandfather_until` pairs it with the
    // `existing_grandfathered` class. Without the repository's own refusal the
    // pairing is discovered by the driver: `RepoError::Db`, `DomainError::Internal`,
    // a 500 for a request whose author only has to clear one field.
    let mut content = flat_content();
    content.grandfather_until = Some(at(23));
    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(price_id, base_key(ChargeKind::Recurring), content.clone()),
        )
        .await
        .expect_err("a horizon on an all_subscriptions key must be refused");
    assert_eq!(
        err,
        RepoError::GrandfatherHorizonOffClass {
            eligibility: "all_subscriptions".to_owned(),
        }
    );

    // D-147 made the pairing a rule, so the refusal reaches a consumer under a
    // code of its own instead of the generic bad-request answer it borrowed
    // while no document stated it. The code is what a client branches on: it
    // names the field to clear, which the shared answer never did.
    let mapped = CanonicalError::from(repo_failure(&err));
    let body = format!("{mapped:?}");
    assert!(body.contains("GRANDFATHER_UNTIL_FORBIDDEN"), "got: {body}");
    assert!(body.contains("grandfather_until"), "got: {body}");
    assert_eq!(
        mapped.status_code(),
        400,
        "an architectural 422 reaches the wire as a 400 carrying its code"
    );

    // The third class may not carry one either: the horizon expires a *retained
    // generation*, and this class retains nobody.
    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                price_id,
                new_subscriptions_key(ChargeKind::Recurring),
                content.clone(),
            ),
        )
        .await
        .expect_err("a horizon on a new_subscriptions_only key must be refused");
    assert_eq!(
        err,
        RepoError::GrandfatherHorizonOffClass {
            eligibility: "new_subscriptions_only".to_owned(),
        }
    );

    // Refused before the transaction opens, so the key is still free.
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None
    );
    let mut clean = flat_content();
    clean.grandfather_until = None;
    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), clean),
    )
    .await
    .expect("clearing the horizon is the whole remedy");

    // The same refusal on the update path, where the class comes from the
    // **stored** row rather than the submitted key — which is why the check
    // cannot be the create path's alone.
    let err = repo
        .update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect_err("a horizon submitted onto a non-grandfathered row must be refused");
    assert_eq!(
        err,
        RepoError::GrandfatherHorizonOffClass {
            eligibility: "all_subscriptions".to_owned(),
        }
    );
    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(
        read.grandfather_until, None,
        "the refused edit left nothing"
    );
    assert_eq!(read.row_version, RowVersion::new(0), "and moved no tag");

    // And the class that may carry one still does.
    let grandfathered = Uuid::from_u128(0xb_5b);
    let mut retained = flat_content();
    retained.grandfather_until = Some(at(23));
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            grandfathered,
            grandfathered_key(ChargeKind::Recurring, at(9)),
            retained,
        ),
    )
    .await
    .expect("a grandfathered generation carries its horizon");
}

#[tokio::test]
async fn an_authored_instant_finer_than_the_quantum_never_reaches_a_column() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_5d);

    // The horizon is authored content and the column would take a finer value in
    // silence, which is exactly how a truncating producer and a non-truncating
    // consumer end up agreeing until the day they do not (D-144).
    let mut content = flat_content();
    content.grandfather_until = Some(at(23) + chrono::TimeDelta::microseconds(1));
    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                price_id,
                grandfathered_key(ChargeKind::Recurring, at(9)),
                content.clone(),
            ),
        )
        .await
        .expect_err("a sub-millisecond horizon must be refused");
    assert!(
        matches!(
            &err,
            RepoError::TimestampPrecisionExceeded { field, .. } if field == "grandfatherUntil"
        ),
        "got: {err:?}"
    );
    let mapped = CanonicalError::from(repo_failure(&err));
    assert!(format!("{mapped:?}").contains("TIMESTAMP_PRECISION_EXCEEDED"));
    assert_eq!(mapped.status_code(), 400);

    // Refused before anything was written, and rounding to the quantum is the
    // whole remedy — the value is not moved on the author's behalf.
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None
    );
    content.grandfather_until = Some(at(23));
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            price_id,
            grandfathered_key(ChargeKind::Recurring, at(9)),
            content,
        ),
    )
    .await
    .expect("an instant on the quantum is storable");

    // The cohort axis is refused by the key itself, one layer earlier: it is
    // matched for equality against an instant another gear produced, so an
    // unquantized generation would build a key nobody can find.
    let err = ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::Generation(at(9) + chrono::TimeDelta::nanoseconds(1)),
    )
    .expect_err("a sub-millisecond cutover cannot become an axis value");
    assert!(matches!(err, DomainError::TimestampPrecisionExceeded(_)));
    assert!(format!("{:?}", CanonicalError::from(err)).contains("TIMESTAMP_PRECISION_EXCEEDED"));
}

#[tokio::test]
async fn a_draft_may_shed_its_bands_and_its_tiered_kind_in_one_edit() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_5c);

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            price_id,
            grandfathered_key(ChargeKind::Usage, at(9)),
            graduated_content(),
        ),
    )
    .await
    .expect("create the tiered row");

    // The edit the parent-side kind guard could have broken: `graduated` with
    // three bands becomes `flat` with none. The guard refuses a row that still
    // carries bands leaving the tiered kinds, so this only works because the
    // band set is replaced **before** the row moves. Written as an ordinary
    // authoring edit rather than as a schema case, because that is what it is.
    let mut content = flat_content();
    content.grandfather_until = Some(at(23));
    repo.update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect("a tiered draft may become a flat one in a single edit");

    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.model_kind, Some(ModelKind::Flat));
    assert!(read.row.bands.is_empty());
    assert!(
        stored_bands(&provider, &scope, price_id).await.is_empty(),
        "and the band rows really are gone"
    );

    // The submitted content said `recurring` and the key says `usage`, and the
    // key wins — silently, because the axis is stored once and an update may not
    // move the key it would have to move to honour the other answer. The create
    // path documents this; the update path did not, and this is what it does.
    assert_eq!(read.row.charge_kind, ChargeKind::Usage);
    assert_eq!(read.scope_key.charge_kind(), ChargeKind::Usage);
}

#[tokio::test]
async fn every_other_axis_that_can_move_is_a_different_key_too() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xb_c0),
            base_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("the baseline row");

    // The four axes the test above leaves standing. An axis dropped from the
    // duplicate check's filter would compare fewer columns than the key has and
    // **over-refuse** — rejecting a row on a genuinely different key as a
    // duplicate — which no test that varies only `chargeKind` and `cohort` can
    // see. `priceOverlay` is the one axis with nothing to vary: every row this
    // gear authors carries `base`, and the column has a CHECK saying so.
    let other_plan = PlanId::new(Uuid::from_u128(0x9_1a5));
    let variants = [
        (
            Uuid::from_u128(0xb_c1),
            other_plan,
            "USD",
            "EU",
            0xfa_5e_u128,
        ),
        (Uuid::from_u128(0xb_c2), plan(), "EUR", "EU", 0xfa_5e),
        (Uuid::from_u128(0xb_c3), plan(), "USD", "US", 0xfa_5e),
        (Uuid::from_u128(0xb_c4), plan(), "USD", "EU", 0xfa_5f),
    ];
    for (price_id, plan_id, currency, region, phase) in variants {
        let key = ScopeKey::new(
            plan_id,
            CurrencyCode::new(currency).expect("three letters"),
            Region::new(region).expect("a non-blank region"),
            PhaseId::new(Uuid::from_u128(phase)),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None,
        )
        .expect("all_subscriptions pairs with cohort none");
        repo.create_draft(&scope, tenant(), draft(price_id, key, flat_content()))
            .await
            .unwrap_or_else(|e| {
                panic!("{currency}/{region} on plan {plan_id} phase {phase:x} is its own key: {e}")
            });
    }
}

#[tokio::test]
async fn the_store_itself_refuses_the_second_draft_the_check_can_only_read_for() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let first = Uuid::from_u128(0xb_35);
    let key = base_key(ChargeKind::Recurring);

    repo.create_draft(&scope, tenant(), draft(first, key, flat_content()))
        .await
        .expect("the first row takes the key");

    // The repository's duplicate check is a read, so two concurrent creators
    // can both pass it; what decides the race is
    // `uq_pricing_price_scope_key_draft`. `uq_pricing_price_scope_key_current`
    // cannot do it — that index is partial over `published` and a draft is
    // invisible to it — so before this index existed both callers committed and
    // landed exactly the second draft the check exists to refuse.
    //
    // The insert goes through the entity rather than the repository, because
    // the repository would refuse it a statement earlier: what is under test is
    // the half that survives losing the race.
    let err = insert_bare_draft(&provider, &scope, Uuid::from_u128(0xb_36))
        .await
        .expect_err("a second draft on one canonical scope key must not land");

    // `SQLite` names the colliding **columns** and Postgres names the index, so
    // the assertion is on the axis list. That difference is also why the
    // repository does not turn this back into `DUPLICATE_SCOPE_KEY` itself:
    // recognizing it means knowing which backend is answering, which is a
    // narrowing owed to the surface layer (see `PriceRepo::create_draft`).
    let message = err.to_string();
    assert!(
        message.contains("UNIQUE constraint failed"),
        "the refusal must be a unique violation, got: {message}"
    );
    // `tenant_id` leads the list and is not one of the eight axes: the index
    // scopes the key to a tenant the way the design set's sibling
    // meter-injectivity index does, so the two spellings of "how far this
    // uniqueness reaches" agree.
    for axis in [
        "pricing_price.tenant_id",
        "pricing_price.plan_id",
        "pricing_price.currency",
        "pricing_price.region",
        "pricing_price.price_overlay",
        "pricing_price.phase",
        "pricing_price.price_eligibility",
        "pricing_price.charge_kind",
        "pricing_price.cohort",
    ] {
        assert!(
            message.contains(axis),
            "the violated index must cover the whole canonical key; {axis} is \
             missing from: {message}"
        );
    }

    // And the winner is untouched.
    assert!(
        repo.find(&scope, tenant(), first)
            .await
            .expect("read")
            .is_some()
    );

    // The positive control, and it is what makes the index's `WHERE
    // lifecycle_state = 'draft'` load-bearing: once the occupant publishes, the
    // key takes a draft again. The two partial indexes are disjoint, which is
    // the state the D-88 supersession unit works in — a successor draft beside
    // the published predecessor it will replace. A non-partial UNIQUE here would
    // have made that state unreachable.
    flip_state(&provider, &scope, first, LifecycleState::Published).await;
    insert_bare_draft(&provider, &scope, Uuid::from_u128(0xb_37))
        .await
        .expect("a published occupant leaves the draft index free");
}

/// Insert a minimal draft row on the suite's base recurring key, straight
/// through the entity — the path a caller that had already passed the
/// repository's read would take.
async fn insert_bare_draft(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) -> Result<(), toolkit_db::secure::ScopeError> {
    let conn = provider.conn().expect("conn");
    let row = price::ActiveModel {
        price_id: Set(price_id),
        tenant_id: Set(tenant()),
        plan_id: Set(plan().get()),
        currency: Set("USD".to_owned()),
        region: Set("EU".to_owned()),
        phase: Set(Uuid::from_u128(0xfa_5e)),
        charge_kind: Set(ChargeKind::Recurring.as_str().to_owned()),
        amount_minor: Set(Some(1_000)),
        model_kind: Set(Some("flat".to_owned())),
        lifecycle_state: Set(LifecycleState::Draft.as_str().to_owned()),
        created_by: Set(Uuid::from_u128(0xac_10)),
        created_at_utc: Set(at(10)),
        ..price::ActiveModel::default()
    };
    price::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)?
        .exec(&conn)
        .await
        .map(|_| ())
}

#[tokio::test]
async fn an_edit_advances_the_tag_and_the_previous_tag_stops_working() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_60);

    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("create");

    let mut content = flat_content();
    content.row.amount_minor = Some(money(2_000));
    let edited = repo
        .update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect("the first edit holds the current version");
    assert_eq!(edited.row.amount_minor, Some(money(2_000)));
    assert_eq!(edited.row_version, RowVersion::new(1));

    // The second writer is the bulk import that read before the interactive
    // edit landed. It is refused, and the refusal names both versions.
    let mut stale = flat_content();
    stale.row.amount_minor = Some(money(3_000));
    let err = repo
        .update_draft(&scope, tenant(), price_id, RowVersion::new(0), stale)
        .await
        .expect_err("a submit against a superseded tag must be refused");
    assert_eq!(
        err,
        RepoError::StaleRowVersion {
            subject: "price row".to_owned(),
            id: price_id.to_string(),
            current: 1,
            submitted: 0,
        }
    );

    // And nothing of the refused edit landed.
    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.amount_minor, Some(money(2_000)));
    assert_eq!(read.row_version, RowVersion::new(1));
}

#[tokio::test]
async fn a_frozen_row_refuses_by_name_and_an_absent_one_is_not_found() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_70);

    // A **banded** row on purpose. The delete path has to remove the children
    // before the parent — the foreign key neither cascades nor nulls — so a row
    // with no bands
    // would let `delete_bands` touch nothing, the band trigger never fire, and
    // the row DELETE's own draft conjunct produce the same `NotDraft`. Only a
    // row that actually has bands can tell the typed refusal apart from the
    // trigger's raw one.
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            price_id,
            grandfathered_key(ChargeKind::Usage, at(9)),
            graduated_content(),
        ),
    )
    .await
    .expect("create");
    flip_state(&provider, &scope, price_id, LifecycleState::Published).await;

    // The submitted version is the row's real one, so only the draft-only
    // conjunct can have failed. The typed refusal is the whole point: the table
    // trigger would answer with a database error carrying no state, and the
    // caller would be told the store is broken.
    let err = repo
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            graduated_content(),
        )
        .await
        .expect_err("a published row is frozen in content");
    assert_eq!(
        err,
        RepoError::NotDraft {
            subject: "price row".to_owned(),
            id: price_id.to_string(),
            state: "published".to_owned(),
        }
    );

    // Deleting it is refused the same way, and — this is the part the band set
    // makes observable — *before* the band table is touched. Without the read
    // that refuses first, `delete_bands` would run against a published parent
    // and the band trigger would answer `RepoError::Db`, so the caller would be
    // told the store is broken rather than that the row is published.
    let err = repo
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(0))
        .await
        .expect_err("only a never-published draft is deletable");
    assert_eq!(
        err,
        RepoError::NotDraft {
            subject: "price row".to_owned(),
            id: price_id.to_string(),
            state: "published".to_owned(),
        }
    );
    assert_eq!(
        stored_bands(&provider, &scope, price_id).await.len(),
        3,
        "a refused delete removes no band"
    );

    let absent = Uuid::from_u128(0xb_71);
    let err = repo
        .update_draft(&scope, tenant(), absent, RowVersion::new(0), flat_content())
        .await
        .expect_err("that row was never authored");
    assert_eq!(
        err,
        RepoError::NotFound {
            subject: "price row".to_owned(),
            id: absent.to_string(),
        }
    );
}

#[tokio::test]
async fn an_update_replaces_the_whole_band_set() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_80);
    let key = grandfathered_key(ChargeKind::Usage, at(9));

    repo.create_draft(&scope, tenant(), draft(price_id, key, graduated_content()))
        .await
        .expect("create");
    assert_eq!(stored_bands(&provider, &scope, price_id).await.len(), 3);

    // A new set, not a merge: two bands where there were three, a different
    // lower bound in the middle, and a different top price. A merge would leave
    // the old middle band behind and the geometry rules would see an overlap
    // nobody authored.
    let mut content = graduated_content();
    content.row.bands = vec![
        TierBand::closed(0, 500, money(30)),
        TierBand::open(500, money(20)),
    ];
    let edited = repo
        .update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect("replace the band set");

    assert_eq!(
        edited.row.bands,
        vec![
            TierBand::closed(0, 500, money(30)),
            TierBand::open(500, money(20)),
        ]
    );
    assert_eq!(edited.row_version, RowVersion::new(1));

    // The band edit moved the **parent's** tag. Bands carry none of their own,
    // so without that two authors editing different bands of one draft would
    // both satisfy `If-Match` and silently interleave.
    let stored = stored_bands(&provider, &scope, price_id).await;
    assert_eq!(stored.len(), 2, "exactly the new bands are there");
    assert!(
        stored.iter().all(|band| band.tenant_id == tenant()),
        "a band's tenant comes from its parent row"
    );
}

#[tokio::test]
async fn an_update_rewrites_every_content_column_and_can_clear_one() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_81);
    let key = grandfathered_key(ChargeKind::Usage, at(9));

    repo.create_draft(&scope, tenant(), draft(price_id, key, graduated_content()))
        .await
        .expect("create");

    // Every content column this kind can carry moves at once, and four
    // previously-set optionals are **cleared**. Both halves matter. An update
    // that submitted content byte-identical to what is stored except for the one
    // field under test would pass with any other column silently dropped from
    // the UPDATE — the write-side twin of a dropped read mapping. And clearing
    // is the whole justification for `update_draft` taking whole content instead
    // of a patch: a per-field `Some`/`None` encoding cannot say "set this back
    // to NULL" at all.
    let mut content = graduated_content();
    content.row.model_kind = Some(ModelKind::Volume);
    content.row.bands = vec![TierBand::open(0, money(7))];
    content.row.meter = Some("api_bytes".to_owned());
    content.row.dimension_key = String::new();
    content.row.billing_granularity = Some(BillingGranularity::PerDay);
    content.row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    content.row.aggregation_function = Some(AggregationFunction::Peak);
    content.row.aggregation_granularity = Some(AggregationGranularity::Day);
    content.row.max_hold_granules = Some(12);
    content.row.tier_qualification_window = None;
    content.row.included_allowance = None;
    content.tax_inclusive = false;
    content.billing_timing = Some("advance".to_owned());
    content.rounding_policy_ref = None;
    content.grandfather_until = Some(at(20));
    content.supersedes_price_id = None;

    repo.update_draft(&scope, tenant(), price_id, RowVersion::new(0), content)
        .await
        .expect("replace the whole content");

    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");

    assert_eq!(read.row.model_kind, Some(ModelKind::Volume));
    assert_eq!(read.row.bands, vec![TierBand::open(0, money(7))]);
    assert_eq!(read.row.meter.as_deref(), Some("api_bytes"));
    // The empty string is the empty-tuple sentinel, not an absent value: the
    // column is NOT NULL DEFAULT '' so the Slice-2 injectivity index collides
    // undimensioned rows instead of treating them as distinct NULLs.
    assert_eq!(read.row.dimension_key, "");
    assert_eq!(
        read.row.billing_granularity,
        Some(BillingGranularity::PerDay)
    );
    assert_eq!(
        read.row.tier_aggregation_window,
        Some(TierAggregationWindow::InvoicePeriod)
    );
    assert_eq!(
        read.row.aggregation_function,
        Some(AggregationFunction::Peak)
    );
    assert_eq!(
        read.row.aggregation_granularity,
        Some(AggregationGranularity::Day)
    );
    assert_eq!(read.row.max_hold_granules, Some(12));
    assert!(!read.tax_inclusive);
    assert_eq!(read.billing_timing.as_deref(), Some("advance"));
    assert_eq!(read.grandfather_until, Some(at(20)));

    // The four that were set and are now NULL.
    assert_eq!(read.row.tier_qualification_window, None);
    assert_eq!(read.row.included_allowance, None);
    assert_eq!(read.rounding_policy_ref, None);
    assert_eq!(read.supersedes_price_id, None);

    // And nothing the update may not move has moved.
    assert_eq!(read.scope_key, grandfathered_key(ChargeKind::Usage, at(9)));
    assert_eq!(read.created_by, Uuid::from_u128(0xac_10));
    assert_eq!(read.created_at_utc, at(10));
    assert_eq!(read.row_version, RowVersion::new(1));
}

#[tokio::test]
async fn an_update_reaches_the_per_kind_money_columns_too() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    // The five content columns the tiered row above cannot legally carry:
    // `package_size` / `package_price_minor` live only on a `package` row
    // (`chk_pricing_price_package_fields_kind`), and `amount_minor` /
    // `quantity_source` / `manual_quantity` belong to the untiered kinds. They
    // reach the UPDATE through the same assignment list, so they need the same
    // proof that the list actually names them.
    let package_id = Uuid::from_u128(0xb_82);
    let mut package = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    package.package_size = Some(1_000);
    package.package_price_minor = Some(money(4_999));
    package.billing_granularity = Some(BillingGranularity::WholeUnit);
    package.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    let package_content = PriceContent {
        row: package,
        tax_inclusive: false,
        billing_timing: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    };
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            package_id,
            base_key(ChargeKind::Usage),
            package_content.clone(),
        ),
    )
    .await
    .expect("create the package row");

    let mut resized = package_content;
    resized.row.package_size = Some(500);
    resized.row.package_price_minor = Some(money(2_999));
    repo.update_draft(&scope, tenant(), package_id, RowVersion::new(0), resized)
        .await
        .expect("re-block the package row");

    let read = repo
        .find(&scope, tenant(), package_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.package_size, Some(500));
    assert_eq!(read.row.package_price_minor, Some(money(2_999)));

    let per_unit_id = Uuid::from_u128(0xb_83);
    let mut per_unit = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::PerUnit));
    per_unit.amount_minor = Some(money(1_500));
    per_unit.quantity_source = Some(QuantitySource::Manual);
    per_unit.manual_quantity = Some(12);
    let per_unit_content = PriceContent {
        row: per_unit,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    };
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            per_unit_id,
            base_key(ChargeKind::Recurring),
            per_unit_content.clone(),
        ),
    )
    .await
    .expect("create the per-unit row");

    // Moving to the seat count clears the fixed quantity in the same write —
    // the two fields answer one question, and a row that kept both would give
    // rating two answers to "how many".
    let mut seated = per_unit_content;
    seated.row.amount_minor = Some(money(2_500));
    seated.row.quantity_source = Some(QuantitySource::SubscriptionSeatCount);
    seated.row.manual_quantity = None;
    repo.update_draft(&scope, tenant(), per_unit_id, RowVersion::new(0), seated)
        .await
        .expect("re-price the per-unit row");

    let read = repo
        .find(&scope, tenant(), per_unit_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.row.amount_minor, Some(money(2_500)));
    assert_eq!(
        read.row.quantity_source,
        Some(QuantitySource::SubscriptionSeatCount)
    );
    assert_eq!(read.row.manual_quantity, None);
}

#[tokio::test]
async fn deleting_a_draft_takes_its_bands_with_it() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_90);
    let key = grandfathered_key(ChargeKind::Usage, at(9));

    repo.create_draft(&scope, tenant(), draft(price_id, key, graduated_content()))
        .await
        .expect("create");

    // Abandoning a draft is a write like any other: a caller working from a
    // read it did not refresh would otherwise discard an edit it never saw.
    let err = repo
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(4))
        .await
        .expect_err("a stale tag must not delete");
    assert_eq!(
        err,
        RepoError::StaleRowVersion {
            subject: "price row".to_owned(),
            id: price_id.to_string(),
            current: 0,
            submitted: 4,
        }
    );
    assert_eq!(
        stored_bands(&provider, &scope, price_id).await.len(),
        3,
        "a refused delete removes no band"
    );

    // The bands go first, inside the transaction: the foreign key declares the
    // default NO ACTION on both backends, so the row cannot leave while its
    // children point at it.
    repo.delete_draft(&scope, tenant(), price_id, RowVersion::new(0))
        .await
        .expect("the current tag deletes");
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None
    );
    assert!(
        stored_bands(&provider, &scope, price_id).await.is_empty(),
        "the band set went with its parent"
    );
}

#[tokio::test]
async fn list_for_plan_filters_by_state_and_orders_stably() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let recurring = Uuid::from_u128(0xb_a1);
    let one_time = Uuid::from_u128(0xb_a2);
    let setup = Uuid::from_u128(0xb_a3);

    for (price_id, charge_kind) in [
        (setup, ChargeKind::OneTimeSetup),
        (one_time, ChargeKind::OneTime),
        (recurring, ChargeKind::Recurring),
    ] {
        let mut content = flat_content();
        content.row = PriceRow::new(charge_kind, Some(ModelKind::Flat));
        content.row.amount_minor = Some(money(1_000));
        repo.create_draft(
            &scope,
            tenant(),
            draft(price_id, base_key(charge_kind), content),
        )
        .await
        .expect("create");
    }
    flip_state(&provider, &scope, one_time, LifecycleState::Published).await;

    // Ascending by `price_id`, whatever order the rows were written in. The
    // list surface's cursor (D-125, G7) needs a total order that does not
    // depend on the plan index's physical layout.
    let drafts = repo
        .list_for_plan(&scope, tenant(), plan(), &[LifecycleState::Draft])
        .await
        .expect("list drafts");
    assert_eq!(
        drafts.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        vec![recurring, setup]
    );

    let published = repo
        .list_for_plan(&scope, tenant(), plan(), &[LifecycleState::Published])
        .await
        .expect("list published");
    assert_eq!(
        published.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        vec![one_time]
    );

    // An empty state set selects nothing. Reading it as "every state" would
    // hand a caller whose filter computed to nothing the whole catalog.
    assert!(
        repo.list_for_plan(&scope, tenant(), plan(), &[])
            .await
            .expect("list nothing")
            .is_empty()
    );
}

#[tokio::test]
async fn another_tenants_price_row_is_invisible_and_unwritable() {
    let (repo, _provider) = harness().await;
    let mine = Uuid::from_u128(0x7e_22);
    let price_id = Uuid::from_u128(0xb_b0);
    let key = grandfathered_key(ChargeKind::Usage, at(9));

    repo.create_draft(
        &AccessScope::for_tenant(tenant()),
        tenant(),
        draft(price_id, key, graduated_content()),
    )
    .await
    .expect("the other tenant authors its row");

    // SQL-level BOLA, the same shape `PinFrontierRepo::read` documents: my
    // scope resolves their row to nothing, whichever tenant id I name. The
    // catalog is commercially sensitive, so the reads fail to `None` and the
    // writes fail to "not found" — never to "forbidden", which would confirm
    // the row exists.
    let scope = AccessScope::for_tenant(mine);
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None
    );
    assert!(
        repo.list_for_plan(&scope, tenant(), plan(), &[LifecycleState::Draft])
            .await
            .expect("list")
            .is_empty()
    );

    let err = repo
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            flat_content(),
        )
        .await
        .expect_err("a foreign draft is not writable");
    assert!(matches!(err, RepoError::NotFound { .. }));

    let err = repo
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(0))
        .await
        .expect_err("a foreign draft is not deletable");
    assert!(matches!(err, RepoError::NotFound { .. }));

    // And the row is untouched for its owner, band set included.
    let owner = AccessScope::for_tenant(tenant());
    let read = repo
        .find(&owner, tenant(), price_id)
        .await
        .expect("read")
        .expect("their row is still there");
    assert_eq!(read.row_version, RowVersion::new(0));
    assert_eq!(read.row.bands.len(), 3);
}

#[tokio::test]
async fn a_price_row_may_not_be_created_into_another_tenant() {
    let (repo, _provider) = harness().await;
    let mine = Uuid::from_u128(0x7e_22);
    let price_id = Uuid::from_u128(0xb_b1);

    // The insert side of the BOLA the test above closes for reads, updates and
    // deletes, and the only side no `WHERE` clause can close: a scoped read
    // filters rows that exist, while a scoped insert has to refuse a row before
    // it does. `scope_with_model` is what refuses it — the `ActiveModel`'s own
    // `tenant_id` is checked against the caller's scope — and without that
    // check a caller could plant a price row inside another tenant's catalog,
    // priced however it liked, and then be unable to see or unwrite it.
    let err = repo
        .create_draft(
            &AccessScope::for_tenant(mine),
            tenant(),
            draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
        )
        .await
        .expect_err("a row may not be created into a tenant the caller is not scoped to");
    let RepoError::Db(detail) = err else {
        panic!("a refused insert scope is a storage failure, not a typed refusal");
    };
    assert!(detail.contains("pricing_price scope"), "got: {detail}");

    // And nothing landed under either tenant — least of all the victim's.
    assert_eq!(
        repo.find(&AccessScope::for_tenant(tenant()), tenant(), price_id)
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        repo.find(&AccessScope::for_tenant(mine), mine, price_id)
            .await
            .expect("read"),
        None
    );
}

#[tokio::test]
async fn a_create_the_band_table_refuses_leaves_no_row_behind() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_c8);

    // A `flat` row carrying a band set. It fails in the one place that makes
    // the transaction observable: the row INSERT succeeds — `flat` is a legal
    // kind and the row satisfies every CHECK on its own table — and the *next*
    // statement is refused, by the band table's structural-exclusivity trigger
    // reading the parent this call has just written.
    let mut content = flat_content();
    content.row.bands = vec![TierBand::closed(0, 100, money(50))];
    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(price_id, base_key(ChargeKind::Recurring), content),
        )
        .await
        .expect_err("bands are forbidden on a flat row");
    let RepoError::Db(detail) = err else {
        panic!("the band table's refusal reaches the caller as a storage failure");
    };
    assert!(
        detail.contains("pricing_price_tier_band"),
        "the refusal must be the band table's, got: {detail}"
    );

    // The claim this file's repository makes is "both tables or neither", and
    // this is the only case that can tell the difference: with the two writes
    // run as plain statements the row above is durable, so the caller is handed
    // a failure while the store holds a `flat` row that a later authoring call
    // would find occupying its scope key, that `list_for_plan` would return,
    // and that publish would judge — a row nobody successfully created.
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None,
        "a refused create must leave nothing behind"
    );
    assert!(
        stored_bands(&provider, &scope, price_id).await.is_empty(),
        "and no band either"
    );

    // The key is free, which is the operational half of the same fact: an
    // author whose first attempt was refused may fix the row and try again.
    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("the refused attempt left the scope key unoccupied");
}

#[tokio::test]
async fn a_granule_bound_no_column_can_hold_is_refused_before_anything_is_written() {
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_c9);

    // Every count on the row is a `u64` in the domain over a **signed**
    // `bigint`, so the top half of the domain range has no storage at all and
    // the refusal is reachable through any of them; `max_hold_granules` is the
    // one this case drives. Checked rather than cast: a cast would turn an
    // impossible bound into a plausible one — `i64::MAX + 1` renders as
    // `i64::MIN` — and hold a sampling gap nobody authored, on a column whose
    // own CHECK demands `>= 1`.
    let mut content = graduated_content();
    let unstorable = u64::try_from(i64::MAX).expect("i64::MAX is a u64") + 1;
    content.row.max_hold_granules = Some(unstorable);

    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                price_id,
                grandfathered_key(ChargeKind::Usage, at(9)),
                content,
            ),
        )
        .await
        .expect_err("a bound past the integer column is not storable");

    // Named to the field, and **not** `CorruptRow`: the number arrived on a
    // request and the author can change it, which is the line between a bad
    // request and an internal fault.
    assert_eq!(
        err,
        RepoError::ValueOutOfRange {
            field: "max_hold_granules".to_owned(),
            value: unstorable.to_string(),
        }
    );

    // Rendered before the transaction opens, so the refusal costs no write at
    // all — the key is still free.
    assert_eq!(
        repo.find(&scope, tenant(), price_id).await.expect("read"),
        None
    );
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            price_id,
            grandfathered_key(ChargeKind::Usage, at(9)),
            graduated_content(),
        ),
    )
    .await
    .expect("a storable bound is authored on the same key");
}
