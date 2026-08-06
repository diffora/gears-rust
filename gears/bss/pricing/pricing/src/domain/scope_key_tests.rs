//! Tests for the canonical scope key and its axes.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    COHORT_ELIGIBILITY_MISMATCH, ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId,
    PriceEligibility, PriceOverlay, Region, ScopeKey, USAGE_LINE_AXIS_MISMATCH,
    check_cohort_eligibility, check_usage_line_axes,
};
use crate::domain::error::DomainError;
use crate::domain::money::CurrencyCode;

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(1))
}

fn phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(2))
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").expect("USD is well-formed")
}

fn eu() -> Region {
    Region::new("EU").expect("EU is a non-blank region")
}

fn cutover() -> DateTime<Utc> {
    DateTime::from_timestamp(1_770_000_000, 0).expect("fixed instant is in range")
}

fn key(
    price_eligibility: PriceEligibility,
    charge_kind: ChargeKind,
    cohort: Cohort,
) -> Result<ScopeKey, DomainError> {
    ScopeKey::new(
        plan(),
        usd(),
        eu(),
        phase(),
        price_eligibility,
        charge_kind,
        cohort,
    )
}

fn violation_codes(err: &DomainError) -> Vec<String> {
    match err {
        DomainError::ValidationFailed(report) => {
            report.violations.iter().map(|v| v.code.clone()).collect()
        }
        other => panic!("expected a validation envelope, got {other}"),
    }
}

#[test]
fn a_cohort_without_grandfathered_eligibility_is_rejected() {
    // Forward direction of the iff. Such a row would sit on a key no
    // resolution class ever selects: published, and never priced from.
    let err = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::Generation(cutover()),
    )
    .expect_err("a cohort on an all_subscriptions row must be refused");

    assert_eq!(violation_codes(&err), vec![COHORT_ELIGIBILITY_MISMATCH]);
}

#[test]
fn grandfathered_eligibility_without_a_cohort_is_rejected() {
    // Reverse direction of the iff, and the more damaging one: the row would
    // land on the all_subscriptions successor's own key and, being immutable,
    // occupy the key the next reprice needs.
    let err = key(
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect_err("a grandfathered row without a generation must be refused");

    assert_eq!(violation_codes(&err), vec![COHORT_ELIGIBILITY_MISMATCH]);
}

#[test]
fn both_consistent_pairings_are_accepted() {
    assert!(
        key(
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None
        )
        .is_ok()
    );
    assert!(
        key(
            PriceEligibility::ExistingGrandfathered,
            ChargeKind::Recurring,
            Cohort::Generation(cutover())
        )
        .is_ok()
    );
}

#[test]
fn a_new_subscriptions_only_row_is_classless_on_the_cohort_axis() {
    // The third class retains nobody, so it takes the `none` cohort like
    // `all_subscriptions` does. Reading the biconditional as "not
    // all_subscriptions implies a generation" would have made the class
    // unauthorable the moment it existed.
    assert!(
        key(
            PriceEligibility::NewSubscriptionsOnly,
            ChargeKind::Recurring,
            Cohort::None
        )
        .is_ok()
    );
    let err = key(
        PriceEligibility::NewSubscriptionsOnly,
        ChargeKind::Recurring,
        Cohort::Generation(cutover()),
    )
    .expect_err("a cohort on a new_subscriptions_only row must be refused");
    assert_eq!(violation_codes(&err), vec![COHORT_ELIGIBILITY_MISMATCH]);
}

#[test]
fn the_three_eligibility_classes_rank_in_the_most_specific_wins_order() {
    // W3 (`07-pricewindow-linkage.md`) and PRD 1.4 order the classes
    // `existing_grandfathered` > `new_subscriptions_only` > `all_subscriptions`.
    // The derived `Ord` follows declaration order, so a variant inserted in the
    // wrong place would leave this type ranking its own classes one way while
    // Tariffs resolves them another — with nothing to say which is the
    // authority.
    assert!(PriceEligibility::AllSubscriptions < PriceEligibility::NewSubscriptionsOnly);
    assert!(PriceEligibility::NewSubscriptionsOnly < PriceEligibility::ExistingGrandfathered);
}

#[test]
fn the_eligibility_tokens_are_the_persisted_ones() {
    // The persisted spelling is also the wire spelling the read model exposes
    // (PRD 6.9), so a token renamed here silently re-classes stored rows.
    assert_eq!(
        PriceEligibility::AllSubscriptions.as_str(),
        "all_subscriptions"
    );
    assert_eq!(
        PriceEligibility::NewSubscriptionsOnly.as_str(),
        "new_subscriptions_only"
    );
    assert_eq!(
        PriceEligibility::ExistingGrandfathered.as_str(),
        "existing_grandfathered"
    );
}

#[test]
fn the_pairing_is_re_checkable_without_a_key() {
    // The two axes come back from storage as two independent columns, so the
    // rehydration path needs the rule without having to build a key first.
    assert!(check_cohort_eligibility(PriceEligibility::AllSubscriptions, Cohort::None).is_ok());
    assert!(
        check_cohort_eligibility(
            PriceEligibility::ExistingGrandfathered,
            Cohort::Generation(cutover())
        )
        .is_ok()
    );
    assert!(check_cohort_eligibility(PriceEligibility::AllSubscriptions, Cohort::None).is_ok());
    assert!(
        check_cohort_eligibility(PriceEligibility::ExistingGrandfathered, Cohort::None).is_err()
    );
}

#[test]
fn a_hybrid_plans_recurring_and_usage_rows_are_distinct_keys() {
    // The reason chargeKind is an axis: one plan legitimately holds both rows
    // on the same currency, region and phase. Without the axis the second one
    // would be rejected as a duplicate of the first.
    let recurring = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("recurring key");
    let usage = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
        Cohort::None,
    )
    .expect("usage key");
    let setup = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::OneTimeSetup,
        Cohort::None,
    )
    .expect("setup key");

    assert_ne!(recurring, usage);
    assert_ne!(recurring, setup);
    assert_ne!(usage, setup);
}

#[test]
fn two_cutovers_on_one_key_produce_two_keys() {
    // Grandfathering is repeatable: each cutover mints its own generation, so
    // the retained windows never contend for one key's non-overlap rule.
    let first = key(
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::Generation(cutover()),
    )
    .expect("first generation");
    let later = DateTime::from_timestamp(1_780_000_000, 0).expect("fixed instant is in range");
    let second = key(
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::Generation(later),
    )
    .expect("second generation");

    assert_ne!(first, second);
}

#[test]
fn an_authored_row_always_carries_the_base_overlay() {
    // Partner / orgTier / brand adjustments are separate PriceOverlay rows
    // evaluated downstream; this axis is never how they reach a price row.
    let authored = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("base key");

    assert_eq!(authored.price_overlay(), PriceOverlay::Base);
}

#[test]
fn the_axis_defaults_are_base_all_subscriptions_and_none() {
    assert_eq!(PriceOverlay::default(), PriceOverlay::Base);
    assert_eq!(
        PriceEligibility::default(),
        PriceEligibility::AllSubscriptions
    );
    assert_eq!(Cohort::default(), Cohort::None);
}

#[test]
fn the_canonical_rendering_carries_the_first_eight_axes_in_order() {
    // The rendering is what a DUPLICATE_SCOPE_KEY rejection names, so a
    // dropped axis would report a collision between rows that do not collide.
    // The usage pair D-196 appends is the ten-axis test's subject; this one
    // owns the order of the eight that are unconditional.
    let rendered = key(
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::OneTime,
        Cohort::Generation(cutover()),
    )
    .expect("key")
    .to_string();

    let axes: Vec<&str> = rendered.split('|').collect();
    assert_eq!(axes[0], plan().to_string());
    assert_eq!(axes[1], "USD");
    assert_eq!(axes[2], "EU");
    assert_eq!(axes[3], "base");
    assert_eq!(axes[4], phase().to_string());
    assert_eq!(axes[5], "existing_grandfathered");
    assert_eq!(axes[6], "one_time");
    assert_eq!(axes[7], cutover().timestamp_millis().to_string());
}

#[test]
fn a_currency_axis_differing_only_in_case_is_the_same_key() {
    // The normalization in CurrencyCode exists for exactly this: otherwise two
    // rows on one market would both pass the duplicate-key index.
    let upper = ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("USD"),
        eu(),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("key");
    let lower = ScopeKey::new(
        plan(),
        CurrencyCode::new("usd").expect("usd"),
        eu(),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("key");

    assert_eq!(upper, lower);
}

#[test]
fn a_blank_region_is_not_an_axis_value() {
    // An empty axis component is not "no region", it is a key that cannot be
    // compared against any other key.
    assert!(Region::new("   ").is_err());
    assert_eq!(Region::new(" EU ").expect("trimmed").as_str(), "EU");
}

#[test]
fn the_charge_kind_tokens_are_the_persisted_ones() {
    assert_eq!(ChargeKind::Recurring.as_str(), "recurring");
    assert_eq!(ChargeKind::Usage.as_str(), "usage");
    assert_eq!(ChargeKind::OneTime.as_str(), "one_time");
    assert_eq!(ChargeKind::OneTimeSetup.as_str(), "one_time_setup");
}

// ---------------------------------------------------------------------------
// The usage line axes (D-196, clause 1)
// ---------------------------------------------------------------------------

fn usage_key(meter: Option<&str>, dimension: &str) -> Result<ScopeKey, DomainError> {
    key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
        Cohort::None,
    )
    .expect("the eight axes agree")
    .with_usage_line(
        meter.map(|m| Meter::new(m).expect("a non-blank meter")),
        DimensionKey::new(dimension),
    )
}

#[test]
fn two_usage_lines_of_one_market_are_two_keys() {
    // The whole of D-196: D-103's confirmed example — one plan pricing
    // cloudlets and egress in one market — rendered ONE key under the eight
    // axes, and the second line was refused DUPLICATE_SCOPE_KEY at save.
    let cloudlets = usage_key(Some("cloudlets"), "").expect("a metered line");
    let egress = usage_key(Some("egress_gb"), "").expect("a second metered line");

    assert_ne!(cloudlets, egress);
    assert_ne!(cloudlets.to_string(), egress.to_string());
}

#[test]
fn one_meter_dimensioned_two_ways_is_two_keys() {
    // `dimensionKey` is the second half of the line and discriminates on its
    // own: the meter-line index has always keyed `(meter, dimension_key)`.
    let eu = usage_key(Some("cloudlets"), "region=eu").expect("a dimensioned line");
    let us = usage_key(Some("cloudlets"), "region=us").expect("a second dimension");

    assert_ne!(eu, us);
}

#[test]
fn a_meter_on_a_non_usage_key_is_refused() {
    // The implication: a meter present means chargeKind = usage. A recurring
    // row carrying one would mint a key the meter-line index never sees.
    let err = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the eight axes agree")
    .with_usage_line(
        Some(Meter::new("cloudlets").expect("meter")),
        DimensionKey::none(),
    )
    .expect_err("a meter on a recurring key is refused");

    assert_eq!(violation_codes(&err), vec![USAGE_LINE_AXIS_MISMATCH]);
}

#[test]
fn a_dimension_key_on_a_non_usage_key_is_refused() {
    // Same implication, reached through the other axis of the pair.
    let err = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::OneTime,
        Cohort::None,
    )
    .expect("the eight axes agree")
    .with_usage_line(None, DimensionKey::new("region=eu"))
    .expect_err("a dimension key on a one_time key is refused");

    assert_eq!(violation_codes(&err), vec![USAGE_LINE_AXIS_MISMATCH]);
}

#[test]
fn a_dimension_key_without_a_meter_is_refused_on_a_usage_key_too() {
    // A dimension discriminates the dimensions OF a meter; with no meter it
    // names nothing, and it would hand the store a second key for the same
    // meterless usage line. Not stated in the design set before this clause —
    // the amendment is recorded on D-196.
    let err = usage_key(None, "region=eu").expect_err("a dimension with no meter is refused");

    assert_eq!(violation_codes(&err), vec![USAGE_LINE_AXIS_MISMATCH]);
}

#[test]
fn a_usage_key_with_no_meter_is_still_admissible() {
    // The rule is an implication and deliberately NOT a biconditional: the
    // meter/usage-type binding (`inst-cmp-usagetype`) is registry-dependent
    // and deferred, so a usage row carrying no meter is a shape the authoring
    // plane accepts today. A biconditional here would refuse it.
    assert!(usage_key(None, "").is_ok());
}

#[test]
fn a_blank_meter_is_not_an_axis_value() {
    // The empty string is the store's sentinel for "no meter" inside
    // COALESCE(meter, ''), so a blank meter would collide with the absent one
    // rather than being its own line.
    assert!(Meter::new("   ").is_err());
    assert_eq!(
        Meter::new(" cloudlets ").expect("trimmed").as_str(),
        "cloudlets"
    );
}

#[test]
fn the_canonical_rendering_carries_all_ten_axes_in_order() {
    // The rendering is what a DUPLICATE_SCOPE_KEY rejection names and what the
    // approval register and the registry idempotency key embed, so the arity
    // is fixed at ten whatever the row is (D-196).
    let rendered = usage_key(Some("cloudlets"), "region=eu")
        .expect("a dimensioned line")
        .to_string();

    let axes: Vec<&str> = rendered.split('|').collect();
    assert_eq!(axes.len(), 10);
    assert_eq!(axes[6], "usage");
    assert_eq!(axes[7], "none");
    assert_eq!(axes[8], "cloudlets");
    assert_eq!(axes[9], "region=eu");
}

#[test]
fn a_non_usage_key_renders_the_sentinel_on_both_usage_axes() {
    // Fixed arity means the pair is rendered even where it cannot be set, and
    // `none` is the token for the same reason `Cohort::None` uses it: an empty
    // segment between two separators cannot be told from a rendering bug.
    let rendered = key(
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("key")
    .to_string();

    let axes: Vec<&str> = rendered.split('|').collect();
    assert_eq!(axes.len(), 10);
    assert_eq!(axes[8], "none");
    assert_eq!(axes[9], "none");
}

#[test]
fn the_usage_line_rule_is_checkable_without_a_key() {
    // `check_cohort_eligibility`'s reason, one axis pair over: the columns are
    // read back from the store as independent values, so the pairing has to be
    // re-established on every rehydration and not only at construction.
    assert!(check_usage_line_axes(ChargeKind::Usage, None, &DimensionKey::none()).is_ok());
    assert!(
        check_usage_line_axes(
            ChargeKind::Recurring,
            Some(&Meter::new("cloudlets").expect("meter")),
            &DimensionKey::none(),
        )
        .is_err()
    );
}
