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

mod common;

use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::{PriceContent, PriceRecord};
use bss_pricing::domain::price_row::{
    AggregationFunction, AggregationGranularity, BillingGranularity, IncludedAllowance, ModelKind,
    PriceRow, QuantitySource, RolloverPolicy, TierAggregationWindow, TierBand,
    TierQualificationWindow,
};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::domain::tax_display::{RegionReadiness, RegionTaxReadiness};
use bss_pricing::infra::storage::entity::{audit_log, price, price_tier_band};
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

/// The readiness a fixture publishes against.
///
/// **Empty deliberately.** This suite is about the row plane, not D-154: an empty
/// lookup freezes `resolved_tax_category` as NULL, which is what a row stating no
/// category in a region declaring no default should carry.
fn fixture_readiness() -> bss_pricing::domain::tax_display::RegionTaxReadiness {
    bss_pricing::domain::tax_display::RegionTaxReadiness::empty()
}

/// One value for a whole test binary: these suites drive a repository or a
/// service directly, where the value the HTTP edge would have established has
/// no producer. What each suite asserts *about* it is stated where it asserts
/// it.
const TEST_CORRELATION: uuid::Uuid = uuid::Uuid::from_u128(0x_c0_11_a7_10);

/// The repository, plus the provider the seeding helper needs to put a row into
/// a state `price_repo::publish_rows` reaches. Both states it fabricates now have
/// real producers — `superseded` gained one with
/// `price_repo::commit_supersession_rows` (D-88's row half) — and it is kept
/// anyway: a case about one row should not have to compose a whole supersession to
/// reach the state it wants to assert against.
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
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
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
        tax_category_ref: None,
        billing_timing: Some("arrears".to_owned()),
        proration_contract: None,
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
        correlation_id: TEST_CORRELATION,
    }
}

/// Move a row's `lifecycle_state` directly.
///
/// `draft -> published` is `price_repo::publish_rows`'s flip and
/// `published -> superseded` is `commit_supersession_rows`'s; both are fabricated
/// here so a suite about one row needs neither a publish unit nor a composed
/// supersession. The append-only trigger permits both: it fires only when the row
/// is already past `draft`, and `published -> superseded` is one of the two flips
/// it whitelists. (The reason stated here used to be "the trigger fires only when the
/// row is already past `draft`", which D-153 falsified: the trigger gained a draft
/// branch on both backends. It still *permits* both flips, so the fabrication is legal
/// — corrected 2026-08-05.)
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
    // **The key the row is filed under carries the line the content named**
    // (D-196 clause 3): `ScopeKeyRequest` has no meter member, so the row's own
    // fields are the author's only way to state the line and the door derives
    // the ninth and tenth axes from them. The key handed in here names no line;
    // the key that comes back does, and it is the same key the two scope-key
    // indexes hold the row under.
    assert_eq!(
        read.scope_key,
        key.clone()
            .with_usage_line(
                Some(Meter::new("api_calls").expect("a non-blank meter")),
                DimensionKey::new("region:eu"),
            )
            .expect("a usage key carries its line")
    );
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
    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(0),
        content,
        stamp(),
    )
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
                tax_category_ref: None,
                billing_timing: None,
                proration_contract: None,
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
                tax_category_ref: None,
                billing_timing: Some("advance".to_owned()),
                proration_contract: None,
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
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            content,
            stamp(),
        )
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
async fn an_authored_instant_finer_than_the_quantum_is_refused_on_both_write_paths() {
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
            content.clone(),
        ),
    )
    .await
    .expect("an instant on the quantum is storable");

    // **And the edit path refuses it too**, which is the half that matters most
    // on this plane: tightening `grandfather_until` on a draft row is the
    // sanctioned authoring move (`inst-gs-tighten`), so it is the likely way a
    // sub-millisecond horizon actually arrives. Guarding only creation would
    // leave the store one `PATCH` away from a column holding an instant finer
    // than the one the catalog compares at — and `timestamptz` takes it in
    // silence, so nothing downstream would ever report it.
    content.grandfather_until = Some(at(20) + chrono::TimeDelta::microseconds(1));
    let err = repo
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            content,
            stamp(),
        )
        .await
        .expect_err("a sub-millisecond horizon must be refused on the edit path too");
    assert!(
        matches!(
            &err,
            RepoError::TimestampPrecisionExceeded { field, .. } if field == "grandfatherUntil"
        ),
        "got: {err:?}"
    );

    // Refused ahead of the compare-and-swap, so the row keeps the horizon it
    // had and its tag never moved — an author who resubmits at the quantum is
    // still holding a current `ETag`.
    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(
        read.grandfather_until,
        Some(at(23)),
        "the refused edit left nothing"
    );
    assert_eq!(read.row_version, RowVersion::new(0), "and moved no tag");

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
    // The row was authored as a metered usage line, and the line is an axis
    // since D-196 clause (3): an edit that dropped it would be asking to move
    // the row to another key, which this door refuses. Carried forward
    // explicitly so this case stays about the band set and the kind.
    content.row.meter = Some("api_calls".to_owned());
    content.row.dimension_key = "region:eu".to_owned();
    content.grandfather_until = Some(at(23));
    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(0),
        content,
        stamp(),
    )
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

    // **This assertion changed shape with D-196 clause (2), and the change is a
    // measured property of `SQLite` rather than a weakening chosen here.** It
    // used to enumerate the nine indexed columns, because `SQLite` names the
    // colliding *columns* while Postgres names the index. Once the index carries
    // an **expression** — `COALESCE(meter, '')`, the sentinel that keeps a
    // nullable `meter` from dissolving the uniqueness of every non-usage key —
    // `SQLite` stops naming columns at all and names the index instead:
    //
    //     UNIQUE (a, b)                     -> "UNIQUE constraint failed: t.a, t.b"
    //     UNIQUE (a, b, COALESCE(meter,'')) -> "UNIQUE constraint failed: index 'ix'"
    //
    // measured directly on both forms, 2026-08-06. So the axis list is no longer
    // available to assert on this engine, and the index **name** is what both
    // engines now have in common. That is a real cost of the sentinel and it is
    // recorded on D-196 rather than absorbed silently: a reader who wants to know
    // which axes the guarantee covers reads the migration, not the error.
    //
    // The rest of the original note stands: this is also why the repository does
    // not turn the violation back into `DUPLICATE_SCOPE_KEY` itself — recognizing
    // it means knowing which backend is answering, a narrowing owed to the
    // surface layer (see `PriceRepo::create_draft`).
    let message = err.to_string();
    assert!(
        message.contains("UNIQUE constraint failed"),
        "the refusal must be a unique violation, got: {message}"
    );
    assert!(
        message.contains("uq_pricing_price_scope_key_draft"),
        "the violated guard must be the draft-plane scope-key index, got: {message}"
    );

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
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            content,
            stamp(),
        )
        .await
        .expect("the first edit holds the current version");
    assert_eq!(edited.row.amount_minor, Some(money(2_000)));
    assert_eq!(edited.row_version, RowVersion::new(1));

    // The second writer is the bulk import that read before the interactive
    // edit landed. It is refused, and the refusal names both versions.
    let mut stale = flat_content();
    stale.row.amount_minor = Some(money(3_000));
    let err = repo
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            stale,
            stamp(),
        )
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
            stamp(),
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
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(0), stamp())
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
        .update_draft(
            &scope,
            tenant(),
            absent,
            RowVersion::new(0),
            flat_content(),
            stamp(),
        )
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
        .update_draft(
            &scope,
            tenant(),
            price_id,
            RowVersion::new(0),
            content,
            stamp(),
        )
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
    // **The line stays put, and that is D-196 clause (3) narrowing this case by
    // exactly two columns.** `meter` and `dimensionKey` are axes of the canonical
    // scope key now, not content, so an update may not move them — the case
    // below asserts the refusal. Everything else here still moves.
    content.row.meter = Some("api_calls".to_owned());
    content.row.dimension_key = "region:eu".to_owned();
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

    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(0),
        content,
        stamp(),
    )
    .await
    .expect("replace the whole content");

    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("present");

    assert_eq!(read.row.model_kind, Some(ModelKind::Volume));
    assert_eq!(read.row.bands, vec![TierBand::open(0, money(7))]);
    // The line is unchanged because an update may not move it (D-196 clause 3);
    // what this case is about is every column that still moves.
    assert_eq!(read.row.meter.as_deref(), Some("api_calls"));
    // The empty string is the empty-tuple sentinel, not an absent value: the
    // column is NOT NULL DEFAULT '' so the Slice-2 injectivity index collides
    // undimensioned rows instead of treating them as distinct NULLs. It is now
    // also the tenth axis's sentinel, which is why `''` had to stay unauthorable
    // as a *meter* — see `Meter::new`.
    assert_eq!(read.row.dimension_key, "region:eu");
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

    // And nothing the update may not move has moved — including the usage line,
    // which is an axis of the key rather than an editable content field since
    // D-196 clause (3), so the update carries it forward rather than re-deriving
    // it.
    assert_eq!(
        read.scope_key,
        grandfathered_key(ChargeKind::Usage, at(9))
            .with_usage_line(
                Some(Meter::new("api_calls").expect("a non-blank meter")),
                DimensionKey::new("region:eu"),
            )
            .expect("a usage key carries its line")
    );
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
        tax_category_ref: None,
        billing_timing: None,
        proration_contract: None,
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
    repo.update_draft(
        &scope,
        tenant(),
        package_id,
        RowVersion::new(0),
        resized,
        stamp(),
    )
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
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
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
    repo.update_draft(
        &scope,
        tenant(),
        per_unit_id,
        RowVersion::new(0),
        seated,
        stamp(),
    )
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
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(4), stamp())
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
    repo.delete_draft(&scope, tenant(), price_id, RowVersion::new(0), stamp())
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
            stamp(),
        )
        .await
        .expect_err("a foreign draft is not writable");
    assert!(matches!(err, RepoError::NotFound { .. }));

    let err = repo
        .delete_draft(&scope, tenant(), price_id, RowVersion::new(0), stamp())
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
        .expect_err("a bound past the signed bigint column is not storable");

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

/// `publish_rows` through a real transaction and over an explicit validated
/// set, which is now what its signature demands.
///
/// The set is `(price_id, row_version)` for every row the publish claims to
/// have judged: the repository publishes exactly those rows at exactly those
/// versions and re-derives nothing, so a row whose content moved between
/// validation and the flip is refused naming the row.
async fn publish_rows(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    validated: Vec<(Uuid, RowVersion)>,
    readiness: &RegionTaxReadiness,
) -> Result<Vec<Uuid>, RepoError> {
    let scope = scope.clone();
    let readiness = readiness.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<Vec<Uuid>, RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::price_repo::publish_rows(
                    txn, &scope, tenant_id, plan_id, &validated, &readiness,
                )
                .await
            })
        })
        .await;
    outcome.map_err(|err| {
        err.into_domain(|infra| RepoError::Db(format!("publish transaction: {infra}")))
    })
}

/// Every `draft` row of the plan, paired with the version it stands at — the
/// shape a publish subject hands the repository after the rule set has passed.
async fn validated_drafts(
    repo: &PriceRepo,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Vec<(Uuid, RowVersion)> {
    repo.list_for_plan(scope, tenant_id, plan_id, &[LifecycleState::Draft])
        .await
        .expect("read the plan's draft rows")
        .into_iter()
        .map(|record| (record.price_id, record.row_version))
        .collect()
}

// ---------------------------------------------------------------------------
// The publish unit's price-row flip.
// ---------------------------------------------------------------------------

/// The stored row, whatever the repository would say about it.
async fn stored_row(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) -> price::Model {
    let conn = provider.conn().expect("conn");
    price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .one(&conn)
        .await
        .expect("read the stored row")
        .expect("the row is there")
}

/// A readiness declaring one region's default category.
fn readiness_for(region: &str, category: Option<&str>) -> RegionTaxReadiness {
    RegionTaxReadiness::new(
        [(
            region.to_owned(),
            RegionReadiness {
                tax_category: category.map(ToOwned::to_owned),
                tax_rate_present: true,
            },
        )]
        .into_iter()
        .collect(),
    )
}

/// **D-154: publish resolves the effective category and freezes the result.**
///
/// The row states no category of its own, so the value comes from the region
/// default — and it is written **by the publish statement**, against the
/// readiness the rule set judged the row with.
///
/// This is `T-13`. It was first built resolving in the projector instead, which
/// runs up to D-47's five-minute batching maximum later, against a region
/// taxonomy anyone holding `config × write` may have re-declared in between: a
/// version could freeze a category no rule ever judged, or lose one that was
/// present when publish passed. Freezing here is what makes the value a property
/// of the publish rather than of whenever the sweep happened to run.
#[tokio::test]
async fn publish_freezes_the_effective_tax_category_from_the_readiness_it_judged_with() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_00c1);
    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the row");

    publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        vec![(price_id, RowVersion::new(0))],
        &readiness_for(
            base_key(ChargeKind::Recurring).region().as_str(),
            Some("standard"),
        ),
    )
    .await
    .expect("publish");

    let conn = provider.conn().expect("conn");
    let frozen = bss_pricing::infra::storage::repo::price_repo::resolved_tax_categories(
        &conn,
        &scope,
        tenant(),
        plan(),
    )
    .await
    .expect("read the frozen categories");

    assert_eq!(
        frozen.get(&price_id).cloned().flatten().as_deref(),
        Some("standard"),
        "the region default resolved at publish is what the row carries"
    );
}

/// The row's **own** category wins, and is what is frozen.
#[tokio::test]
async fn publish_freezes_the_rows_own_category_over_the_region_default() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_00c2);
    let mut content = flat_content();
    content.tax_category_ref = Some("reduced".to_owned());
    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), content),
    )
    .await
    .expect("author the row");

    publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        vec![(price_id, RowVersion::new(0))],
        &readiness_for(
            base_key(ChargeKind::Recurring).region().as_str(),
            Some("standard"),
        ),
    )
    .await
    .expect("publish");

    let conn = provider.conn().expect("conn");
    let frozen = bss_pricing::infra::storage::repo::price_repo::resolved_tax_categories(
        &conn,
        &scope,
        tenant(),
        plan(),
    )
    .await
    .expect("read");

    assert_eq!(
        frozen.get(&price_id).cloned().flatten().as_deref(),
        Some("reduced"),
        "D-110 makes the row the source of truth; the default is only a fallback"
    );
}

#[tokio::test]
async fn draft_rows_flip_and_published_rows_are_left_alone() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let already = Uuid::from_u128(0xb_0001);
    let pending = Uuid::from_u128(0xb_0002);
    repo.create_draft(
        &scope,
        tenant(),
        draft(already, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the first row");
    flip_state(&provider, &scope, already, LifecycleState::Published).await;
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            pending,
            new_subscriptions_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("author the second row");

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    let moved = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("publish the plan's draft rows");

    assert_eq!(moved, vec![pending], "only the draft row moved");
    assert_eq!(
        stored_row(&provider, &scope, pending).await.lifecycle_state,
        LifecycleState::Published.as_str()
    );
    let untouched = stored_row(&provider, &scope, already).await;
    assert_eq!(
        untouched.lifecycle_state,
        LifecycleState::Published.as_str()
    );
    assert_eq!(
        untouched.row_version, 0,
        "an already-published row is not re-published and its tag does not move"
    );
}

#[tokio::test]
async fn the_published_rows_entity_tag_freezes_with_the_content_it_names() {
    // D-141 / §3.7: the tag joins the frozen whitelist, and this flip changes no
    // content. A bump here would move an entity tag under a representation no
    // caller can write to.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_0001);
    let authored = repo
        .create_draft(
            &scope,
            tenant(),
            draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
        )
        .await
        .expect("author the row");

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("publish");

    let published = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read it back")
        .expect("it is there");
    assert_eq!(published.lifecycle_state, LifecycleState::Published);
    assert_eq!(published.row_version, authored.row_version);
}

#[tokio::test]
async fn the_key_moves_from_the_draft_plane_index_to_the_published_plane() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let first = Uuid::from_u128(0xb_0001);
    repo.create_draft(
        &scope,
        tenant(),
        draft(first, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the row");

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("publish");

    // The draft-plane index (D-148) released the key as the row left `draft`,
    // and the published-plane index claimed it as the row arrived — so a second
    // draft on the same key is now refused by `create_draft`'s occupancy check
    // rather than by either index.
    let refusal = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                Uuid::from_u128(0xb_0002),
                base_key(ChargeKind::Recurring),
                flat_content(),
            ),
        )
        .await
        .expect_err("the key is occupied by a published row");
    assert!(
        matches!(refusal, RepoError::DuplicateScopeKey(_)),
        "got {refusal:?}"
    );
}

// ---------------------------------------------------------------------------
// The supersession door (D-195), and the ordering its commit owes.
// ---------------------------------------------------------------------------

/// `insert_successor_draft_on` through a real transaction, the way the D-88
/// composer will hold it.
async fn supersede(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    draft: NewPriceDraft,
) -> Result<(PriceRecord, Uuid), RepoError> {
    let scope = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<(PriceRecord, Uuid), RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::price_repo::insert_successor_draft_on(
                    txn, &scope, tenant_id, draft,
                )
                .await
            })
        })
        .await;
    outcome.map_err(|err| {
        err.into_domain(|infra| RepoError::Db(format!("supersession transaction: {infra}")))
    })
}

/// Put a published row on the base key and answer its id — the predecessor every
/// case below supersedes.
async fn published_predecessor(
    repo: &PriceRepo,
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
) -> Uuid {
    let predecessor = Uuid::from_u128(0xb_5001);
    repo.create_draft(
        &scope.clone(),
        tenant(),
        draft(predecessor, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the predecessor");
    flip_state(provider, scope, predecessor, LifecycleState::Published).await;
    predecessor
}

#[tokio::test]
async fn the_supersession_door_puts_a_successor_draft_on_the_key_its_predecessor_holds() {
    // The shape §3.7's two disjoint partial `UNIQUE`s permit and the authoring
    // door refuses: a draft beside the published row it will supersede, on one
    // canonical scope key. D-195 clause (2) — the occupancy precondition is the
    // authoring door's, inverted.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let predecessor = published_predecessor(&repo, &provider, &scope).await;
    let successor = Uuid::from_u128(0xb_5002);

    let (record, superseded) = supersede(
        &provider,
        &scope,
        tenant(),
        draft(successor, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("a published occupant is this door's precondition, not its refusal");

    assert_eq!(superseded, predecessor, "the door names what it superseded");
    assert_eq!(record.price_id, successor);
    assert_eq!(record.lifecycle_state, LifecycleState::Draft);
    // Both rows stand on the key, which is the whole point.
    assert_eq!(
        stored_row(&provider, &scope, predecessor)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "the predecessor is untouched by the compose - the flip is the commit's"
    );
    assert_eq!(
        stored_row(&provider, &scope, successor)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str()
    );
}

#[tokio::test]
async fn the_supersession_door_stamps_the_predecessor_it_found() {
    // D-127: the successor carries `supersedes_price_id`. The door stamps it from
    // the same read that validated the key, so a link disagreeing with the key's
    // actual current row is not expressible — whatever the caller sent.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let predecessor = published_predecessor(&repo, &provider, &scope).await;
    let successor = Uuid::from_u128(0xb_5002);

    let mut content = flat_content();
    content.supersedes_price_id = Some(Uuid::from_u128(0xdead_beef));
    let (record, _) = supersede(
        &provider,
        &scope,
        tenant(),
        draft(successor, base_key(ChargeKind::Recurring), content),
    )
    .await
    .expect("compose");

    assert_eq!(
        record.supersedes_price_id,
        Some(predecessor),
        "the door's own read decides the link, not the payload"
    );
    assert_eq!(
        stored_row(&provider, &scope, successor)
            .await
            .supersedes_price_id,
        Some(predecessor),
        "and it is what the table holds"
    );
}

#[tokio::test]
async fn the_supersession_door_refuses_a_key_no_published_row_holds() {
    // `inst-su-compose` presupposes current coverage and fails compose on a
    // dormant key. This is that presupposition read off the row plane: a key with
    // no current row has nothing to supersede, and the caller named a target that
    // is not there.
    let (_repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    let err = supersede(
        &provider,
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xb_5002),
            base_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect_err("an empty key is not supersedable");

    let RepoError::NotFound { subject, id } = err else {
        panic!("a key with no current row must answer NotFound, got: {err:?}");
    };
    assert!(subject.contains("current price"), "got: {subject}");
    assert!(
        id.starts_with(&base_key(ChargeKind::Recurring).to_string()),
        "the refusal names the key that has no occupant, got: {id}"
    );
}

#[tokio::test]
async fn the_supersession_door_refuses_a_key_a_draft_already_stands_on() {
    // One draft per key stays the most the two doors admit between them (§3.7's
    // D-148 argument, under D-195's two-door reading). A draft here is a
    // composition that already staged one; `inst-co-single-pending` refuses the
    // second *unit* a layer up, and this is the floor under it.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    published_predecessor(&repo, &provider, &scope).await;
    let first = Uuid::from_u128(0xb_5002);
    let second = Uuid::from_u128(0xb_5003);

    supersede(
        &provider,
        &scope,
        tenant(),
        draft(first, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("the first composition stages its successor");

    let err = supersede(
        &provider,
        &scope,
        tenant(),
        draft(second, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect_err("a second successor on one key must be refused");

    let RepoError::DuplicateScopeKey(detail) = err else {
        panic!("a draft occupant must answer DUPLICATE_SCOPE_KEY, got: {err:?}");
    };
    assert!(detail.contains("draft"), "got: {detail}");
    assert!(
        detail.contains(&first.to_string()),
        "the refusal names the draft holding the key, got: {detail}"
    );
}

#[tokio::test]
async fn a_successor_publishes_only_after_its_predecessor_leaves_the_published_plane() {
    // D-195 clause (3), measured rather than reasoned. §3.7 admits one published
    // row per key, so the order of the commit's two row moves is not free: the
    // failing order is a raw driver error - a 500 - and not a refusal, which is
    // why the rule is written down at `inst-su-commit` rather than left to be
    // rediscovered. With the flip first, `publish_rows` needs no change at all.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let predecessor = published_predecessor(&repo, &provider, &scope).await;
    let successor = Uuid::from_u128(0xb_5002);
    supersede(
        &provider,
        &scope,
        tenant(),
        draft(successor, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("stage the shape through the door that permits it");

    let validated = vec![(successor, RowVersion::new(0))];
    let refused = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated.clone(),
        &fixture_readiness(),
    )
    .await
    .expect_err("publishing beside a live predecessor collides on the key");
    let RepoError::Db(detail) = &refused else {
        panic!("the collision arrives as a storage fault, got: {refused:?}");
    };
    assert!(
        detail.contains("UNIQUE"),
        "and it is the published-plane index that produced it, got: {detail}"
    );

    // The ordering `inst-su-commit` now states: the predecessor leaves first.
    flip_state(&provider, &scope, predecessor, LifecycleState::Superseded).await;
    let moved = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("with the key free on the published plane, the flip is ordinary");

    assert_eq!(moved, vec![successor]);
    assert_eq!(
        stored_row(&provider, &scope, successor)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str()
    );
}

/// `commit_supersession_rows` through a real transaction — the row half of
/// `inst-su-commit`, whose whole point is that the caller does not order the two
/// moves.
async fn commit_supersession_rows(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    predecessor: Uuid,
    successor: (Uuid, RowVersion),
) -> Result<(), RepoError> {
    let scope = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::price_repo::commit_supersession_rows(
                    txn,
                    &scope,
                    tenant_id,
                    plan_id,
                    predecessor,
                    successor,
                    &fixture_readiness(),
                )
                .await
            })
        })
        .await;
    outcome.map_err(|err| {
        err.into_domain(|infra| RepoError::Db(format!("supersession commit transaction: {infra}")))
    })
}

/// A key carrying its published predecessor and the staged successor draft — the
/// world `inst-su-commit` runs against.
async fn composed_supersession(
    repo: &PriceRepo,
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
) -> (Uuid, Uuid) {
    let predecessor = published_predecessor(repo, provider, scope).await;
    let successor = Uuid::from_u128(0xb_5002);
    supersede(
        provider,
        scope,
        tenant(),
        draft(successor, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("compose");
    (predecessor, successor)
}

#[tokio::test]
async fn the_supersession_commit_flips_the_predecessor_and_publishes_the_successor() {
    // `inst-su-commit`'s two row moves, in the one order that works, from one
    // call — so that no caller is in a position to order them wrongly. D-195
    // clause (3) in code rather than in prose.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor) = composed_supersession(&repo, &provider, &scope).await;

    commit_supersession_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        predecessor,
        (successor, RowVersion::new(0)),
    )
    .await
    .expect("the pair commits");

    assert_eq!(
        stored_row(&provider, &scope, predecessor)
            .await
            .lifecycle_state,
        LifecycleState::Superseded.as_str(),
        "the predecessor left the published plane"
    );
    assert_eq!(
        stored_row(&provider, &scope, successor)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "and the successor arrived on it"
    );
}

#[tokio::test]
async fn the_predecessors_flip_leaves_its_frozen_entity_tag_alone() {
    // D-141 / §3.7: the row version freezes with the published row's content and
    // neither sanctioned in-place mutation moves it — not the `lifecycle_state`
    // flips, not the monotonic `grandfatherUntil` tightening. A tag that moved
    // under a representation no caller can write to would report a stale cache
    // that is not stale.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor) = composed_supersession(&repo, &provider, &scope).await;
    let before = stored_row(&provider, &scope, predecessor).await.row_version;

    commit_supersession_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        predecessor,
        (successor, RowVersion::new(0)),
    )
    .await
    .expect("the pair commits");

    assert_eq!(
        stored_row(&provider, &scope, predecessor).await.row_version,
        before,
        "the flip is not a content mutation and does not move the tag"
    );
}

#[tokio::test]
async fn the_supersession_commit_refuses_a_predecessor_that_is_no_longer_published() {
    // The replay case, and the reason the flip is a compare-and-swap rather than
    // an UPDATE the state machine is trusted to have gated: a committed unit
    // leaves the key's former current row `superseded`, and a second commit of
    // the same unit must not silently move a row that has already moved.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor) = composed_supersession(&repo, &provider, &scope).await;
    flip_state(&provider, &scope, predecessor, LifecycleState::Superseded).await;

    let err = commit_supersession_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        predecessor,
        (successor, RowVersion::new(0)),
    )
    .await
    .expect_err("a row that has already left `published` is not supersedable again");

    // Not `NotDraft`: that variant's sentence names as the remedy an operation
    // this caller is not attempting. The remedy here is to recompose against the
    // key's new current row.
    let RepoError::NotSupersedable { id, state, .. } = err else {
        panic!("the refusal names the state it found, got: {err:?}");
    };
    assert_eq!(id, predecessor.to_string());
    assert_eq!(state, LifecycleState::Superseded.as_str());
}

#[tokio::test]
async fn the_supersession_commit_leaves_the_predecessor_standing_when_the_successor_will_not_publish()
 {
    // `inst-su-commit`: "or everything rolls back". The predecessor's flip is the
    // first move, so a successor refused by the *second* is exactly the case that
    // would leave a key with no current row at all — a sales outage produced by a
    // half-applied unit. The successor is refused on a version that moved.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor) = composed_supersession(&repo, &provider, &scope).await;

    let err = commit_supersession_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        predecessor,
        (successor, RowVersion::new(7)),
    )
    .await
    .expect_err("a successor whose content moved since validation is refused");
    assert!(
        matches!(err, RepoError::StaleRowVersion { .. }),
        "the successor's own precondition still decides, got: {err:?}"
    );

    assert_eq!(
        stored_row(&provider, &scope, predecessor)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "the flip rolled back with the publish it was paired with"
    );
    assert_eq!(
        stored_row(&provider, &scope, successor)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str()
    );
}

#[tokio::test]
async fn a_plan_with_no_draft_rows_publishes_nothing_and_says_so() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    let moved = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("an empty plan is not an error");

    assert!(moved.is_empty());
}

#[tokio::test]
async fn another_tenants_draft_rows_are_invisible_to_a_publish() {
    let (repo, provider) = harness().await;
    let mine = AccessScope::for_tenant(tenant());
    let theirs_tenant = Uuid::from_u128(0x7e_22);
    let theirs = AccessScope::for_tenant(theirs_tenant);
    let my_row = Uuid::from_u128(0xb_0001);
    let their_row = Uuid::from_u128(0xb_0002);
    repo.create_draft(
        &mine,
        tenant(),
        draft(my_row, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author mine");
    repo.create_draft(
        &theirs,
        theirs_tenant,
        draft(
            their_row,
            new_subscriptions_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("author theirs");

    let validated = validated_drafts(&repo, &mine, tenant(), plan()).await;
    let moved = publish_rows(
        &provider,
        &mine,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("publish my rows");

    assert_eq!(moved, vec![my_row]);
    assert_eq!(
        stored_row(&provider, &theirs, their_row)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str(),
        "SecureORM keeps another tenant's rows out of this plan's publish"
    );
}

// ---------------------------------------------------------------------------
// The validated set is the set, and nothing else publishes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_row_whose_content_moved_since_validation_is_refused_by_its_own_tag() {
    // The defect this closes: the publish commit validates its subject, makes a
    // network round-trip to the `CatalogVersion` registry, and only then flips.
    // `in_transaction` opens the engine default - READ COMMITTED on Postgres -
    // so every statement takes a fresh snapshot and a concurrent `update_draft`
    // committing inside that window changes the content of a row the rule set
    // already passed. A re-derived draft set would publish the mutation. The
    // row's own entity tag is what refuses it (D-141), naming the row.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_0001);
    repo.create_draft(
        &scope,
        tenant(),
        draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the row");

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    assert_eq!(validated, vec![(price_id, RowVersion::new(0))]);

    // The world moves: the row's content changes and its tag advances with it.
    let mut edited = flat_content();
    edited.row.amount_minor = Some(money(4_242));
    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(0),
        edited,
        stamp(),
    )
    .await
    .expect("edit the draft");

    let refusal = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect_err("a row that moved since validation must not publish");

    assert!(
        matches!(refusal, RepoError::StaleRowVersion { current, submitted, .. }
            if current == 1 && submitted == 0),
        "got {refusal:?}"
    );
    assert_eq!(
        stored_row(&provider, &scope, price_id)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str(),
        "nothing published"
    );
}

#[tokio::test]
async fn a_row_authored_after_validation_is_not_published_by_this_commit() {
    // The other half of the same window: a `create_draft` committing between the
    // subject's assembly and the flip inserts a row the rule set never saw. It
    // is simply not in the set, so it stays `draft` and publishes with the next
    // revision - correct by construction, because nothing validated it.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let judged = Uuid::from_u128(0xb_0001);
    let unjudged = Uuid::from_u128(0xb_0002);
    repo.create_draft(
        &scope,
        tenant(),
        draft(judged, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author the judged row");

    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            unjudged,
            new_subscriptions_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("author the row nobody judged");

    let moved = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect("the validated row publishes");

    assert_eq!(moved, vec![judged]);
    assert_eq!(
        stored_row(&provider, &scope, unjudged)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str(),
        "a row the rule set never saw must not become consumer-visible"
    );
}

#[tokio::test]
async fn a_row_deleted_since_validation_refuses_the_whole_publish() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let kept = Uuid::from_u128(0xb_0001);
    let dropped = Uuid::from_u128(0xb_0002);
    for (price_id, key) in [
        (kept, base_key(ChargeKind::Recurring)),
        (dropped, new_subscriptions_key(ChargeKind::Recurring)),
    ] {
        repo.create_draft(&scope, tenant(), draft(price_id, key, flat_content()))
            .await
            .expect("author");
    }
    let validated = validated_drafts(&repo, &scope, tenant(), plan()).await;
    repo.delete_draft(&scope, tenant(), dropped, RowVersion::new(0), stamp())
        .await
        .expect("discard one of them");

    let refusal = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect_err("a validated row that is gone must refuse the publish");

    assert!(
        matches!(refusal, RepoError::NotFound { .. }),
        "got {refusal:?}"
    );
    assert_eq!(
        stored_row(&provider, &scope, kept).await.lifecycle_state,
        LifecycleState::Draft.as_str(),
        "the transaction took the other row's flip back with it"
    );
}

#[tokio::test]
async fn a_validated_row_of_another_plan_is_caught_by_the_count() {
    // The count assertion's one reachable arm without concurrency, and the only
    // thing that tests it at all. The pre-read is scoped by tenant and identity
    // — it mirrors `mutable_draft`, which does not filter `plan_id` — so a
    // validated entry naming a draft row of another plan of the same tenant
    // passes it. The UPDATE's own plan filter then excludes that row, and the
    // number that moved is one short of the number validated. Without the count
    // the publish would report success having published fewer rows than it was
    // handed.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let mine = Uuid::from_u128(0xb_0001);
    let elsewhere = Uuid::from_u128(0xb_0002);
    let other_plan = PlanId::new(Uuid::from_u128(0x9_1a5));

    repo.create_draft(
        &scope,
        tenant(),
        draft(mine, base_key(ChargeKind::Recurring), flat_content()),
    )
    .await
    .expect("author a row of the plan under publish");

    let foreign_key = ScopeKey::new(
        other_plan,
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");
    repo.create_draft(
        &scope,
        tenant(),
        draft(elsewhere, foreign_key, flat_content()),
    )
    .await
    .expect("author a row of a different plan of the same tenant");

    // Both rows are `draft` at version 0, so both clear the pre-read; only one
    // of them belongs to the plan being published.
    let validated = vec![(mine, RowVersion::new(0)), (elsewhere, RowVersion::new(0))];
    let refusal = publish_rows(
        &provider,
        &scope,
        tenant(),
        plan(),
        validated,
        &fixture_readiness(),
    )
    .await
    .expect_err("a validated set naming a foreign plan's row must not report success");

    assert!(
        refusal
            .to_string()
            .contains("validated 2 price rows and 1 moved"),
        "the refusal must name the shortfall, got {refusal:?}"
    );
    assert_eq!(
        stored_row(&provider, &scope, mine).await.lifecycle_state,
        LifecycleState::Draft.as_str(),
        "and the row that did move rolled back with the transaction"
    );
}

#[tokio::test]
async fn the_keyset_page_walks_the_same_total_order_the_list_declares() {
    // The caller `list_for_plan_filters_by_state_and_orders_stably` was written
    // for. D-125 forbids offset pagination over an append-only store, so the
    // page has to be a keyset walk on the `price_id ASC` order that test pins —
    // and this is where the two are shown to be the same order rather than two
    // orders that happen to agree today.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let ids = [
        Uuid::from_u128(0xbb01),
        Uuid::from_u128(0xbb02),
        Uuid::from_u128(0xbb03),
        Uuid::from_u128(0xbb04),
    ];

    for (price_id, charge_kind) in [
        (ids[3], ChargeKind::OneTimeSetup),
        (ids[1], ChargeKind::OneTime),
        (ids[0], ChargeKind::Recurring),
        (ids[2], ChargeKind::Usage),
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

    // The whole result, in one page, is exactly what the unbounded list gives.
    let whole = repo
        .list_for_plan(&scope, tenant(), plan(), &[LifecycleState::Draft])
        .await
        .expect("list");
    let paged = repo
        .list_for_plan_page(
            &scope,
            tenant(),
            plan(),
            &[LifecycleState::Draft],
            None,
            100,
        )
        .await
        .expect("page");
    assert_eq!(
        paged.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        whole.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        "the page and the list must not be two orders"
    );

    // `limit` bounds the page; `after` resumes STRICTLY after the key, so the
    // row the cursor names is never handed out twice.
    let first = repo
        .list_for_plan_page(&scope, tenant(), plan(), &[LifecycleState::Draft], None, 2)
        .await
        .expect("first page");
    assert_eq!(
        first.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        vec![ids[0], ids[1]]
    );

    let second = repo
        .list_for_plan_page(
            &scope,
            tenant(),
            plan(),
            &[LifecycleState::Draft],
            Some(ids[1]),
            2,
        )
        .await
        .expect("second page");
    assert_eq!(
        second.iter().map(|row| row.price_id).collect::<Vec<_>>(),
        vec![ids[2], ids[3]],
        "`after` is exclusive, or a walk repeats one row on every page boundary"
    );

    // Past the end there is nothing, and an empty state set still selects
    // nothing - the page inherits both of `list_for_plan`'s contracts.
    assert!(
        repo.list_for_plan_page(
            &scope,
            tenant(),
            plan(),
            &[LifecycleState::Draft],
            Some(ids[3]),
            2,
        )
        .await
        .expect("past the end")
        .is_empty()
    );
    assert!(
        repo.list_for_plan_page(&scope, tenant(), plan(), &[], None, 2)
            .await
            .expect("no states")
            .is_empty()
    );

    // The bands travel with the row on the paged path too: a page that dropped
    // them would answer a geometry no rule could evaluate.
    let banded = paged
        .iter()
        .find(|row| row.price_id == ids[0])
        .expect("the recurring row is in the page");
    assert_eq!(banded.row.charge_kind, ChargeKind::Recurring);
}

/// The actor and instant every mutating repository call now records (D-135 - the
/// audit row commits inside the mutation's own transaction).
fn stamp() -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: uuid::Uuid::from_u128(0xac_10),
        recorded_at: chrono::Utc::now(),
        correlation_id: TEST_CORRELATION,
    }
}

/// The same stamp under a named correlation.
fn stamp_correlated(correlation_id: Uuid) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        correlation_id,
        ..stamp()
    }
}

/// **Every price-plane record carries the correlation its caller supplied**
/// (D-178).
///
/// The whole authoring plane's correlation was unconstrained: replacing both
/// `draft.correlation_id` and `stamp.correlation_id` with a fresh
/// `Uuid::now_v7()` at all four sites in `price_repo` left the suite green.
/// Clause (1) - never NULL - is type-enforced by `NewPriceDraft` and
/// `AuditStamp` taking a bare `Uuid`, so it survives any mint; clause (2) has no
/// equality to break on this plane, because no price route writes two records in
/// one call. `rest_plans.rs::two_records_of_one_patch_carry_one_correlation_id`
/// covers the plan plane exactly that way and cannot be repeated here.
///
/// So the binding is taken where the correlation is an **input** rather than an
/// edge-established value: the repository is what answers, and the three
/// mutations are driven under three **distinct** correlations. A per-record mint
/// fails on all three; a record that borrowed a neighbouring call's value fails
/// on the pair it confused. Blanking to `None` fails at the type level and never
/// reaches here.
///
/// The bulk-import arm (D-118 / D-177) is the reason this matters beyond
/// tidiness: it is one call authoring many rows, and it is the first place a
/// per-record mint becomes an untraceable provenance instead of a redundancy.
#[tokio::test]
async fn every_price_record_carries_the_correlation_its_caller_supplied() {
    const CREATED_BY_CALL: Uuid = Uuid::from_u128(0x_c0_11_00_01);
    const EDITED_BY_CALL: Uuid = Uuid::from_u128(0x_c0_11_00_02);
    const DELETED_BY_CALL: Uuid = Uuid::from_u128(0x_c0_11_00_03);

    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xb_c0);

    let mut created = draft(price_id, base_key(ChargeKind::Recurring), flat_content());
    created.correlation_id = CREATED_BY_CALL;
    repo.create_draft(&scope, tenant(), created)
        .await
        .expect("create the draft row");

    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(0),
        flat_content(),
        stamp_correlated(EDITED_BY_CALL),
    )
    .await
    .expect("edit it");

    repo.delete_draft(
        &scope,
        tenant(),
        price_id,
        RowVersion::new(1),
        stamp_correlated(DELETED_BY_CALL),
    )
    .await
    .expect("take it away");

    let conn = provider.conn().expect("conn");
    let written: Vec<(String, Option<Uuid>)> = audit_log::Entity::find()
        .secure()
        .scope_with(&scope)
        .order_by(audit_log::Column::Seq, sea_orm::Order::Asc)
        .all(&conn)
        .await
        .expect("read the trail")
        .into_iter()
        .map(|row| (row.action, row.correlation_id))
        .collect();

    assert_eq!(
        written,
        vec![
            ("create".to_owned(), Some(CREATED_BY_CALL)),
            ("update".to_owned(), Some(EDITED_BY_CALL)),
            ("delete".to_owned(), Some(DELETED_BY_CALL)),
        ],
        "three calls, three records, and each names the call that wrote it"
    );
}

#[tokio::test]
async fn a_grandfathered_generation_may_not_be_superseded() {
    // Foundation §4.3 is normative: "An `existing_grandfathered` row is **immutable in
    // price** and MUST NOT be superseded", restated in S7 §1.7's UC table ("Attempt to
    // supersede or reprice an `existing_grandfathered` row → rejected; only tightening
    // `grandfatherUntil` is allowed"). Found by review 2026-08-05: **nothing enforced
    // it.** The unit guard compares `PriceRow` fields and a `PriceRow` carries no
    // eligibility class; compose reads only intervals; this door checked occupancy only.
    //
    // It is money rather than tidiness, and the two halves compound. The successor
    // rewrites the retained cohort's price — the exact thing the class exists to
    // prevent. And compose would hand `adjust_effective_to` a shorten of that
    // generation's window to the changeover, while that function does **not** enforce
    // D-04's `inst-co-bounds` (coverage through `grandfatherUntil` plus the longest
    // billing cycle) — a gap recorded in its own doc — so a bound subscriber is
    // stranded mid-cycle with no guard on either side.
    //
    // The door is the cheapest correct home: it already holds the key.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let generation = grandfathered_key(ChargeKind::Recurring, at(12));
    let retained = Uuid::from_u128(0xb_5101);
    repo.create_draft(
        &scope,
        tenant(),
        draft(retained, generation.clone(), flat_content()),
    )
    .await
    .expect("author the grandfathered copy");
    flip_state(&provider, &scope, retained, LifecycleState::Published).await;

    let err = supersede(
        &provider,
        &scope,
        tenant(),
        draft(Uuid::from_u128(0xb_5102), generation, flat_content()),
    )
    .await
    .expect_err("a retained generation is immutable in price");

    let RepoError::NotSupersedable { state, .. } = err else {
        panic!("the class refusal names why, got: {err:?}");
    };
    assert!(
        state.contains("existing_grandfathered"),
        "the refusal names the class, got: {state}"
    );
}

// ---------------------------------------------------------------------------
// D-196 clause (3): the repository carries the usage line into and out of keys
// ---------------------------------------------------------------------------

/// The line a key names, as the authoring door receives it.
fn usage_key(meter: Option<&str>, dimension: &str) -> ScopeKey {
    base_key(ChargeKind::Usage)
        .with_usage_line(
            meter.map(|m| Meter::new(m).expect("a non-blank meter")),
            DimensionKey::new(dimension),
        )
        .expect("a usage key carries its line")
}

/// A usage row's content, carrying the same line its key does.
fn usage_line_content(meter: Option<&str>, dimension: &str) -> PriceContent {
    let mut content = flat_content();
    content.row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    content.row.amount_minor = Some(money(1_000));
    content.row.meter = meter.map(std::borrow::ToOwned::to_owned);
    dimension.clone_into(&mut content.row.dimension_key);
    content
}

#[tokio::test]
async fn two_usage_lines_of_one_market_both_author() {
    // D-103's confirmed example, through the door that refused it: the second
    // line used to answer `DUPLICATE_SCOPE_KEY` because both rendered one key.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xd1_96_01),
            usage_key(Some("cloudlets"), ""),
            usage_line_content(Some("cloudlets"), ""),
        ),
    )
    .await
    .expect("the first meter takes its key");

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xd1_96_02),
            usage_key(Some("egress_gb"), ""),
            usage_line_content(Some("egress_gb"), ""),
        ),
    )
    .await
    .expect("a second meter is a second key, which is the whole of D-196");
}

#[tokio::test]
async fn the_occupancy_read_finds_a_meterless_occupant() {
    // **The filter carries the same NULL trap the index did, one layer up.**
    // A key with no meter renders `meter IS NULL`, and `Column::Meter.eq(None)`
    // is `meter = NULL`, which matches nothing — so the occupancy read would
    // answer "free" over an occupied key and the duplicate would be caught by
    // the index as a driver error rather than by the door as a refusal.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xd1_96_03),
            usage_key(None, ""),
            usage_line_content(None, ""),
        ),
    )
    .await
    .expect("the meterless line takes its key");

    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                Uuid::from_u128(0xd1_96_04),
                usage_key(None, ""),
                usage_line_content(None, ""),
            ),
        )
        .await
        .expect_err("a second meterless line on one key must be refused");

    assert!(
        matches!(err, RepoError::DuplicateScopeKey(_)),
        "the door refuses it by name, not the index by driver error: {err:?}"
    );
}

#[tokio::test]
async fn a_metered_line_does_not_occupy_the_meterless_key() {
    // The other direction of the same filter: the two are different keys, so
    // neither read may find the other.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xd1_96_05),
            usage_key(Some("cloudlets"), ""),
            usage_line_content(Some("cloudlets"), ""),
        ),
    )
    .await
    .expect("the metered line takes its own key");

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0xd1_96_06),
            usage_key(None, ""),
            usage_line_content(None, ""),
        ),
    )
    .await
    .expect("the meterless key is free");
}

#[tokio::test]
async fn a_loaded_key_carries_the_line_it_was_filed_under() {
    // `to_scope_key` rebuilds from the columns, so a round trip has to return
    // the ninth and tenth axes or every consumer of a loaded key — the window
    // plane, the approval register, the supersession door — compares keys that
    // are equal on eight axes and different rows.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xd1_96_07);

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            price_id,
            usage_key(Some("cloudlets"), "region=eu"),
            usage_line_content(Some("cloudlets"), "region=eu"),
        ),
    )
    .await
    .expect("author the dimensioned line");

    let loaded = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("the row is there");

    assert_eq!(
        loaded.scope_key.meter().map(Meter::as_str),
        Some("cloudlets")
    );
    assert_eq!(loaded.scope_key.dimension_key().as_str(), "region=eu");
    assert_eq!(loaded.scope_key, usage_key(Some("cloudlets"), "region=eu"));
}

#[tokio::test]
async fn a_content_naming_a_line_its_key_does_not_is_refused() {
    // **A refusal, deliberately, where `charge_kind` gets a rewrite.**
    // `authored_content` rewrites `charge_kind` from the key because the wire
    // cannot express it — a placeholder is forced. The wire *can* express a
    // meter, so a disagreement is a caller's mistake worth naming rather than
    // one to paper over. And a silent rewrite here would be worse than untidy:
    // it would make the D-82 unit guard's `meter` and `dimensionKey` clauses
    // unreachable, which is exactly how `charge_kind`'s placeholder cost three
    // Criticals on 2026-08-06.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    let err = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                Uuid::from_u128(0xd1_96_08),
                usage_key(Some("cloudlets"), ""),
                usage_line_content(Some("egress_gb"), ""),
            ),
        )
        .await
        .expect_err("a row whose meter is not its key's must not be stored under either");

    let message = format!("{err:?}");
    assert!(
        message.contains("cloudlets") && message.contains("egress_gb"),
        "the refusal names both lines so the author can see which is wrong: {message}"
    );
}

#[tokio::test]
async fn an_update_may_not_move_the_row_to_another_line() {
    // **The defect D-196 clause (3) exposed, pinned.** `update_draft` rewrote
    // `meter` and `dimension_key` as ordinary content columns. Once the pair
    // became key axes, that meant a `PATCH` could move a draft onto a *different*
    // canonical scope key with no occupancy check anywhere on the path — the key
    // another row might already hold — and the only thing that would notice is
    // the partial `UNIQUE`, arriving as a driver error rather than as a refusal.
    //
    // The remedy this door names is the one its own doc already named for every
    // other axis: delete the draft and author another one.
    let (repo, _provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0xd1_96_09);

    let created = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                price_id,
                usage_key(Some("cloudlets"), ""),
                usage_line_content(Some("cloudlets"), ""),
            ),
        )
        .await
        .expect("author the metered line");

    let err = repo
        .update_draft(
            &scope,
            tenant(),
            price_id,
            created.row_version,
            usage_line_content(Some("egress_gb"), ""),
            stamp(),
        )
        .await
        .expect_err("an update may not move the row's line");

    let message = format!("{err:?}");
    assert!(
        message.contains("cloudlets") && message.contains("egress_gb"),
        "the refusal names the stored line and the submitted one: {message}"
    );

    // And the row is untouched — the refusal is before the write.
    let read = repo
        .find(&scope, tenant(), price_id)
        .await
        .expect("read")
        .expect("the row survives a refused update");
    assert_eq!(read.scope_key.meter().map(Meter::as_str), Some("cloudlets"));
    assert_eq!(read.row_version, created.row_version);
}

// ---------------------------------------------------------------------------
// `PriceCreated` — the producer S3 puts on this door
// ---------------------------------------------------------------------------

async fn outbox_events(provider: &DBProvider<DbError>, scope: &AccessScope) -> Vec<String> {
    let conn = provider.conn().expect("conn");
    bss_pricing::infra::storage::entity::outbox::Entity::find()
        .secure()
        .scope_with(scope)
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        .map(|row| row.event_name)
        .collect()
}

#[tokio::test]
async fn authoring_a_draft_row_emits_price_created() {
    // **S3 puts the producer here in as many words**: "a draft price row is
    // authored on the canonical scope key ... `PriceCreated` emits per row", and
    // "`PriceCreated` on row authoring". The event has been declared and
    // producerless since the gear was created — every consumer counting row
    // creations has been counting zero.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());

    repo.create_draft(
        &scope,
        tenant(),
        draft(
            Uuid::from_u128(0x9c_01),
            base_key(ChargeKind::Recurring),
            flat_content(),
        ),
    )
    .await
    .expect("author the row");

    assert_eq!(outbox_events(&provider, &scope).await, vec!["PriceCreated"]);
}

#[tokio::test]
async fn editing_and_deleting_a_draft_emit_nothing_further() {
    // The event is `PriceCreated`, not `PriceTouched`: it fires once, on the act
    // that brought the row into existence. The audit chain is what carries the
    // edits, and it already does.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let price_id = Uuid::from_u128(0x9c_02);

    let created = repo
        .create_draft(
            &scope,
            tenant(),
            draft(price_id, base_key(ChargeKind::Recurring), flat_content()),
        )
        .await
        .expect("author the row");
    let mut edited = flat_content();
    edited.row.amount_minor = Some(money(2_000));
    repo.update_draft(
        &scope,
        tenant(),
        price_id,
        created.row_version,
        edited,
        stamp(),
    )
    .await
    .expect("edit it");

    assert_eq!(
        outbox_events(&provider, &scope).await,
        vec!["PriceCreated"],
        "one creation, one event"
    );
}

// ---------------------------------------------------------------------------
// The cutover's row plane (`inst-co-supersede`, D-100)
// ---------------------------------------------------------------------------

async fn cutover_rows(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    predecessor: Uuid,
    successor: (Uuid, RowVersion),
    copy: (Uuid, RowVersion),
    cutover_at: DateTime<Utc>,
) -> Result<(), RepoError> {
    let scope = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::price_repo::commit_cutover_rows(
                    txn,
                    &scope,
                    tenant(),
                    plan(),
                    predecessor,
                    successor,
                    copy,
                    cutover_at,
                    &fixture_readiness(),
                )
                .await
            })
        })
        .await;
    outcome.map_err(|err| err.into_domain(|infra| RepoError::Db(format!("cutover rows: {infra}"))))
}

/// A published predecessor, a successor drafted on its own key, and a copy drafted
/// on the generation `cutover_at` mints.
///
/// **The instant is a parameter because the copy's key is built from it.** It was a
/// constant until 2026-08-06, which left the cross-plane case seeding a copy on the
/// 2026 generation while committing at a 2099 one — an incoherence no assertion
/// could see until `refuse_ungenerational` began comparing the cohort against the
/// act's own instant, and then it reddened that case immediately.
async fn seeded_cutover(
    repo: &PriceRepo,
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    meter: Option<&str>,
    cutover_at: DateTime<Utc>,
) -> (Uuid, (Uuid, RowVersion), (Uuid, RowVersion)) {
    let key = usage_key(meter, "");
    let predecessor = Uuid::from_u128(0xc0_01);
    repo.create_draft(
        scope,
        tenant(),
        draft(predecessor, key.clone(), usage_line_content(meter, "")),
    )
    .await
    .expect("author the predecessor");
    flip_state(provider, scope, predecessor, LifecycleState::Published).await;

    let successor = Uuid::from_u128(0xc0_02);
    let mut content = usage_line_content(meter, "");
    content.supersedes_price_id = Some(predecessor);
    let conn = provider.conn().expect("conn");
    let (authored, _) = bss_pricing::infra::storage::repo::price_repo::insert_successor_draft_on(
        &conn,
        scope,
        tenant(),
        draft(successor, key.clone(), content),
    )
    .await
    .expect("stage the successor on the predecessor's key");

    let copy_id = Uuid::from_u128(0xc0_03);
    let copy_key = bss_pricing::domain::cutover::grandfathered_copy_key(&key, cutover_at, &[])
        .expect("a fresh generation");
    let copied = repo
        .create_draft(
            scope,
            tenant(),
            draft(copy_id, copy_key, usage_line_content(meter, "")),
        )
        .await
        .expect("author the grandfathered copy");

    (
        predecessor,
        (successor, authored.row_version),
        (copy_id, copied.row_version),
    )
}

#[tokio::test]
async fn the_cutover_flips_the_predecessor_and_publishes_both_new_rows() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor, copy) =
        seeded_cutover(&repo, &provider, &scope, Some("cloudlets"), at(20)).await;

    cutover_rows(&provider, &scope, predecessor, successor, copy, at(20))
        .await
        .expect("the cutover's four row moves commit together");

    let state = async |id: Uuid| {
        repo.find(&scope, tenant(), id)
            .await
            .expect("read")
            .expect("present")
            .lifecycle_state
    };
    assert_eq!(state(predecessor).await, LifecycleState::Superseded);
    assert_eq!(state(successor.0).await, LifecycleState::Published);
    assert_eq!(
        state(copy.0).await,
        LifecycleState::Published,
        "the copy publishes in the same transaction, on its own generation key"
    );
}

#[tokio::test]
async fn a_copy_that_is_not_a_generation_of_the_predecessors_market_is_refused() {
    // The looser of the two pairings, and the one that has to be written down:
    // the copy is compared modulo `priceEligibility` and `cohort`, so every other
    // axis — the usage line included, since D-196 — has to be the predecessor's.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor, _) =
        seeded_cutover(&repo, &provider, &scope, Some("cloudlets"), at(20)).await;

    let stranger = Uuid::from_u128(0xc0_04);
    let other_line = bss_pricing::domain::cutover::grandfathered_copy_key(
        &usage_key(Some("egress_gb"), ""),
        at(20),
        &[],
    )
    .expect("a generation of another line");
    let authored = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                stranger,
                other_line,
                usage_line_content(Some("egress_gb"), ""),
            ),
        )
        .await
        .expect("author a generation of a different meter");

    let err = cutover_rows(
        &provider,
        &scope,
        predecessor,
        successor,
        (stranger, authored.row_version),
        at(20),
    )
    .await
    .expect_err("a generation of another line is not this cutover's copy");

    let message = format!("{err:?}");
    assert!(
        message.contains("not a grandfathered generation"),
        "the refusal names what the copy failed to be: {message}"
    );
}

#[tokio::test]
async fn a_successor_on_another_line_of_one_market_is_refused() {
    // **`0586ff4ee`'s owed test, and it was not paid by `a032befd5` as that commit
    // claimed.** That commit varied the *copy* and left `refuse_mispaired` — the
    // guard whose columns it had just widened from eight to ten — driven only by
    // successors on the predecessor's own key. So reverting `scope_key_columns` to
    // eight reddened nothing, which is the whole failure mode a fix's test exists to
    // rule out. This case is the missing one: a successor on a **different meter of
    // the same market**, naming the predecessor, so the key comparison is the only
    // thing left that can refuse it.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, _, copy) =
        seeded_cutover(&repo, &provider, &scope, Some("cloudlets"), at(20)).await;

    let other_line = Uuid::from_u128(0xc0_05);
    let mut content = usage_line_content(Some("egress_gb"), "");
    content.supersedes_price_id = Some(predecessor);
    let authored = repo
        .create_draft(
            &scope,
            tenant(),
            draft(other_line, usage_key(Some("egress_gb"), ""), content),
        )
        .await
        .expect("author a successor on another line of the same market");

    let err = cutover_rows(
        &provider,
        &scope,
        predecessor,
        (other_line, authored.row_version),
        copy,
        at(20),
    )
    .await
    .expect_err("a row on another line is not this key's successor");

    let message = format!("{err:?}");
    assert!(
        message.contains("on a different canonical scope key"),
        "the ten-column comparison is what refuses, not the supersedes link: {message}"
    );
}

#[tokio::test]
async fn a_copy_on_an_earlier_generation_of_the_same_market_is_refused() {
    // The copy is a generation of the right market and carries the right class, and
    // it is still not **this** cutover's copy: its cohort names another instant, so
    // it is a previous cutover's immutable retained row. Publishing it would republish
    // a generation nobody composed, and `inst-co-copy` mints exactly one generation
    // per act — keyed by the act's own instant.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let (predecessor, successor, _) =
        seeded_cutover(&repo, &provider, &scope, Some("cloudlets"), at(20)).await;

    let earlier = Uuid::from_u128(0xc0_06);
    let earlier_key = bss_pricing::domain::cutover::grandfathered_copy_key(
        &usage_key(Some("cloudlets"), ""),
        at(19),
        &[],
    )
    .expect("a generation of the same market at another instant");
    let authored = repo
        .create_draft(
            &scope,
            tenant(),
            draft(
                earlier,
                earlier_key,
                usage_line_content(Some("cloudlets"), ""),
            ),
        )
        .await
        .expect("author an earlier generation of the same market");

    let err = cutover_rows(
        &provider,
        &scope,
        predecessor,
        successor,
        (earlier, authored.row_version),
        at(20),
    )
    .await
    .expect_err("a generation on another instant is another cutover's copy");

    let message = format!("{err:?}");
    assert!(
        message.contains("generation"),
        "the refusal names the axis that is wrong: {message}"
    );
}

#[tokio::test]
async fn the_cross_plane_commit_moves_three_windows_and_three_rows_together() {
    // `inst-gc-commit`: the predecessor's window shortens to the cutover, the
    // successor's and the copy's open there, and the three row moves ride the same
    // transaction. Every instant comes from the one `ComposedCutover`, so the
    // handover is gap-free by construction rather than by a later check.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    // Inside the fixture coverage window `[2099-08-04, 2099-09-01)`, because this
    // case commits against a real window plane. The copy is seeded on **this**
    // instant's generation: the row-plane cases can use any instant, but here the
    // window the shorten moves and the cohort the copy carries are two halves of
    // one act and cannot be built from two different clocks.
    let cutover_at = common::coverage_from() + chrono::Duration::days(3);
    let (predecessor, successor, copy) =
        seeded_cutover(&repo, &provider, &scope, Some("cloudlets"), cutover_at).await;

    let conn = provider.conn().expect("conn");
    let covering =
        common::schedule_coverage_window(&conn, &scope, tenant(), predecessor, stamp()).await;
    let plane = vec![bss_pricing::domain::supersession::NamedWindow {
        window_id: covering.window_id,
        interval: bss_pricing::domain::window::WindowInterval::new(
            covering.effective_from,
            covering.effective_to,
            covering.state,
        ),
    }];
    let composed = bss_pricing::domain::cutover::compose_cutover_windows(&plane, cutover_at)
        .expect("a live key composes");

    let plan = bss_pricing::infra::cutover::CutoverCommit::of_composition(
        composed,
        plan(),
        predecessor,
        covering.mutation_seq,
        successor,
        Uuid::from_u128(0xc0_10),
        copy,
        Uuid::from_u128(0xc0_11),
        "grandfatheringCutover".to_owned(),
    );

    let scope_for_txn = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<_, RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::cutover::commit_cutover(
                    txn,
                    &scope_for_txn,
                    tenant(),
                    plan,
                    stamp(),
                )
                .await
            })
        })
        .await;
    let written = outcome
        .map_err(|err| err.into_domain(|infra| RepoError::Db(format!("cutover: {infra}"))))
        .expect("the cutover commits across both planes");

    assert_eq!(written.shortened.effective_to, Some(cutover_at));
    assert_eq!(written.successor_window.effective_from, cutover_at);
    assert_eq!(
        written.copy_window.effective_from, cutover_at,
        "both arrivals open at the instant the predecessor's coverage ends"
    );
    assert_eq!(
        written.copy_window.effective_to, None,
        "the copy's window is open-ended, which is what makes the D-04 bound hold"
    );

    let state = async |id: Uuid| {
        repo.find(&scope, tenant(), id)
            .await
            .expect("read")
            .expect("present")
            .lifecycle_state
    };
    assert_eq!(state(predecessor).await, LifecycleState::Superseded);
    assert_eq!(state(successor.0).await, LifecycleState::Published);
    assert_eq!(state(copy.0).await, LifecycleState::Published);
}

// ---------------------------------------------------------------------------
// D-246 — the catalog-wide GA backlog.
// ---------------------------------------------------------------------------

/// A publishable market key on one region, everything else held constant.
fn market_key(region: &str) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none")
}

/// A grandfathered generation's key **on a market of its own**.
///
/// The market matters and the first version of the case got it wrong: seeding
/// the frozen generation on `EU`, which two live rows already gate, made the
/// exclusion unobservable — removing it from the query changed no count, so the
/// clause was asserted by a fixture that could not reach the state it claimed to
/// cover. Found by a probe that reddened **nothing**.
fn grandfathered_market_key(region: &str, cutover: DateTime<Utc>) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::Generation(cutover),
    )
    .expect("existing_grandfathered pairs with a generation")
}

/// The flat recurring shape, priced **tax-inclusive** — the gated one.
fn tax_inclusive_flat() -> PriceContent {
    let mut content = flat_content();
    content.tax_inclusive = true;
    content
}

/// The backlog counts **markets**, deduplicated, and excludes everything a
/// re-publish could never clear (D-246).
///
/// D-246/D-250: the refresher publishes the catalog-wide count to the gauge.
///
/// It lives here rather than beside the job because the seeding does: every row is
/// written through `create_draft` and flipped, so the catalog the job reads is one
/// the gear could actually produce. The job's own module tests what it owns without
/// a catalog — that a failed read publishes nothing, and that the tick is the
/// configured one — and deliberately builds no `ActiveModel` by hand.
#[tokio::test]
async fn the_refresher_publishes_the_catalog_wide_gated_market_count() {
    use bss_pricing::config::JobsConfig;
    use bss_pricing::domain::ports::metrics::{
        CurrencyBindingCase, PreviewFailClosed, PricingAlarm, PricingMetricsPort,
    };
    use bss_pricing::infra::jobs::gated_markets::GatedMarketsJob;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    #[derive(Default)]
    struct Recording(AtomicI64);
    impl PricingMetricsPort for Recording {
        fn preview_failclosed(&self, _reason: PreviewFailClosed) {}
        fn currency_binding_block(&self, _case: CurrencyBindingCase) {}
        fn tax_not_sellable_ga(&self, count: i64) {
            self.0.store(count, Ordering::Relaxed);
        }
        fn alarm(&self, _alarm: PricingAlarm) {}
    }

    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let metrics = Arc::new(Recording::default());
    let job = GatedMarketsJob::new(
        provider.clone(),
        Arc::clone(&metrics) as Arc<dyn PricingMetricsPort>,
        JobsConfig::default(),
    );

    // Two published tax-inclusive rows on one market, and a third on another: the
    // count dedups, so the gauge must read 2 rather than 3.
    for (n, key) in [
        (0xd3_01_u128, market_key("EU")),
        (0xd3_02, new_subscriptions_key(ChargeKind::Recurring)),
        (0xd3_03, market_key("US")),
    ] {
        let price_id = Uuid::from_u128(n);
        repo.create_draft(&scope, tenant(), draft(price_id, key, tax_inclusive_flat()))
            .await
            .expect("create");
        flip_state(&provider, &scope, price_id, LifecycleState::Published).await;
    }

    let report = job.run_once().await.expect("the pass reads");

    assert_eq!(report.gated_markets, 2, "three rows, two markets");
    assert_eq!(
        metrics.0.load(Ordering::Relaxed),
        2,
        "and the gauge carries what the pass read, not a per-plan contribution"
    );
}

/// Every exclusion here is a separate seeded row rather than a clause in a
/// comment, because the count is a single number and a wrong one is
/// indistinguishable from a right one without saying which rows it is made of.
#[tokio::test]
async fn the_gated_market_count_dedups_markets_and_excludes_what_cannot_be_cleared() {
    use bss_pricing::infra::storage::repo::price_repo;

    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let all = AccessScope::allow_all();
    let conn = provider.conn().expect("conn");

    // A real zero before anything is published — the control without which every
    // assertion below would also pass against a count that always answered 0.
    assert_eq!(
        price_repo::gated_markets(&conn, &all)
            .await
            .expect("count an empty catalog"),
        0
    );

    // Two published tax-inclusive rows on **one** market: `all_subscriptions`
    // and `new_subscriptions_only` are different keys and the same market.
    for (n, key) in [
        (0xd2_01_u128, market_key("EU")),
        (0xd2_02, new_subscriptions_key(ChargeKind::Recurring)),
    ] {
        let price_id = Uuid::from_u128(n);
        repo.create_draft(&scope, tenant(), draft(price_id, key, tax_inclusive_flat()))
            .await
            .expect("create");
        flip_state(&provider, &scope, price_id, LifecycleState::Published).await;
    }
    assert_eq!(
        price_repo::gated_markets(&conn, &all).await.expect("count"),
        1,
        "two rows on one market are one market"
    );

    // A second market.
    let second = Uuid::from_u128(0xd2_03);
    repo.create_draft(
        &scope,
        tenant(),
        draft(second, market_key("US"), tax_inclusive_flat()),
    )
    .await
    .expect("create");
    flip_state(&provider, &scope, second, LifecycleState::Published).await;
    assert_eq!(
        price_repo::gated_markets(&conn, &all).await.expect("count"),
        2
    );

    // Three rows that must **not** count, seeded one at a time so a regression
    // names which exclusion broke.
    //
    // A grandfathered generation: immutable, MUST NOT be superseded, so a market
    // reached only through one can never be un-gated by re-publishing (ADR-0002).
    let frozen = Uuid::from_u128(0xd2_04);
    repo.create_draft(
        &scope,
        tenant(),
        draft(
            frozen,
            grandfathered_market_key("GF", at(9)),
            tax_inclusive_flat(),
        ),
    )
    .await
    .expect("create");
    flip_state(&provider, &scope, frozen, LifecycleState::Published).await;
    assert_eq!(
        price_repo::gated_markets(&conn, &all).await.expect("count"),
        2,
        "a grandfathered generation is in no backlog any action can clear"
    );

    // Tax-**exclusive**: sellable today, so not gated at all.
    let exclusive = Uuid::from_u128(0xd2_05);
    repo.create_draft(
        &scope,
        tenant(),
        draft(exclusive, market_key("AP"), flat_content()),
    )
    .await
    .expect("create");
    flip_state(&provider, &scope, exclusive, LifecycleState::Published).await;
    assert_eq!(
        price_repo::gated_markets(&conn, &all).await.expect("count"),
        2,
        "the gate is on the tax basis, and this row does not carry it"
    );

    // A draft: nothing is published on that market, so nothing is gated on it.
    let unpublished = Uuid::from_u128(0xd2_06);
    repo.create_draft(
        &scope,
        tenant(),
        draft(unpublished, market_key("LA"), tax_inclusive_flat()),
    )
    .await
    .expect("create");
    assert_eq!(
        price_repo::gated_markets(&conn, &all).await.expect("count"),
        2,
        "a draft has published nothing and gates nothing"
    );
}
