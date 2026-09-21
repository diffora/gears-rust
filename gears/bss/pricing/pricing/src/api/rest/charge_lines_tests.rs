//! The two doors' wire conversions, judged without a database.
//!
//! What a route case cannot isolate: which half of a resolved row each request
//! shape produces, and which refusals are decided in the conversion rather than by
//! the store. Every case here builds a request the way a client would and reads
//! back the `PriceContent` the repository would be handed.

use super::*;
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x_a1a2))
}

fn key_request(charge_kind: &str) -> LineScopeKeyRequest {
    LineScopeKeyRequest {
        phase: Uuid::from_u128(0x_a4a5),
        sku_id: Uuid::from_u128(0x_50c0),
        price_eligibility: "all_subscriptions".to_owned(),
        charge_kind: charge_kind.to_owned(),
        cohort: None,
        dimension_key: None,
    }
}

fn tiered() -> StructureView {
    StructureView {
        model_kind: Some("graduated".to_owned()),
        ..StructureView::default()
    }
}

/// A two-band ladder: `[0, 100)` then the open top.
fn ladder(rates: [i64; 2]) -> Vec<TierView> {
    vec![
        TierView {
            from_qty: 0,
            to_qty: Some(100),
            rate_nano_minor: rates[0],
        },
        TierView {
            from_qty: 100,
            to_qty: None,
            rate_nano_minor: rates[1],
        },
    ]
}

/// A line record standing in for one the repository would have read.
fn line_record(structure: &StructureView) -> LineRecord {
    let key = line_key_of(plan(), &key_request("usage")).expect("the axes parse");
    let content = structure_content(&key, structure).expect("the structure converts");
    LineRecord {
        charge_line_id: Uuid::from_u128(0x11e0),
        line_version_id: Uuid::from_u128(0x11e2),
        plan_revision: 0,
        lifecycle_state: LifecycleState::Draft,
        row_version: RowVersion::new(0),
        scope_key: key,
        content,
        resolved_invoice_line_template: None,
        resolved_gl_code: None,
        created_by: Uuid::nil(),
        created_at_utc: OffsetDateTime::UNIX_EPOCH,
    }
}

/// **A structure carries no money, whatever it is converted through — and a
/// ladder is money.**
///
/// The conversion renders a structure as the flat content it is the shared half of
/// so that one reader parses every token. A structure that arrived carrying an
/// amount, or a band, would be indistinguishable from one that did not, which is
/// the confusion this pins.
#[test]
fn a_converted_structure_has_no_ladder_and_no_money() {
    let key = line_key_of(plan(), &key_request("usage")).expect("the axes parse");
    let content = structure_content(&key, &tiered()).expect("the structure converts");

    assert!(content.row.bands.is_empty(), "a ladder is a market's");
    assert!(content.row.amount_minor.is_none());
    assert!(content.row.unit_rate.is_none());
    assert!(content.row.package_price_minor.is_none());
    assert!(content.row.reserved_rate.is_none());
    assert!(!content.tax_inclusive);
    assert!(content.tax_category_ref.is_none());
    assert!(content.rounding_policy_ref.is_none());
    assert!(content.grandfather_until.is_none());
}

/// An authored meter is refused **including an explicit `null`**, which a plain
/// `Option` would fold into "absent" and let through.
#[test]
fn an_authored_meter_is_refused_on_a_structure() {
    let key = line_key_of(plan(), &key_request("usage")).expect("the axes parse");
    let refused = structure_content(
        &key,
        &StructureView {
            meter: Some("cloudlets".to_owned()),
            ..StructureView::default()
        },
    );
    let Err(DomainError::ValidationFailed(report)) = refused else {
        panic!("an authored meter must be refused: {refused:?}");
    };
    assert_eq!(
        report.violations.len(),
        1,
        "one refusal, and it is this one: {report:?}"
    );
    assert_eq!(
        report.violations[0].subject, "structure.meter",
        "and it names the member the caller sent: {report:?}"
    );
}

/// The eight axes come off the request; the two monetary ones are not members of it.
#[test]
fn the_line_key_is_the_eight_structural_axes() {
    let key = line_key_of(plan(), &key_request("one_time")).expect("the axes parse");

    assert_eq!(key.plan_id(), plan());
    assert_eq!(key.charge_kind(), ChargeKind::OneTime);
    assert_eq!(key.price_overlay().as_str(), "base");
    assert!(
        key.dimension_key().is_none(),
        "an absent dimension is absent"
    );
    assert_eq!(
        key.to_string().split('|').count(),
        8,
        "eight segments, and no currency among them: {key}"
    );
}

/// An unknown token is a refusal here, not a stored value the publish finds later.
#[test]
fn an_unknown_axis_token_is_refused_at_the_door() {
    let refused = line_key_of(plan(), &key_request("setup_fee"));
    assert!(
        matches!(refused, Err(DomainError::InvalidRequest(_))),
        "a charge kind outside the three is refused: {refused:?}"
    );
}

/// No ladder at all is an unfinished draft, which a draft is allowed to be.
#[test]
fn a_market_with_no_ladder_yet_converts() {
    let line = line_record(&tiered());
    let content = market_content(&line, &MoneyView::default(), &MarketPolicyView::default())
        .expect("an unpriced market is a legal draft");
    assert!(content.row.bands.is_empty(), "no ladder, so no bands");
}

/// **The market's ladder lands whole: its own bounds, its own rates.** Nothing
/// of it is the line's to supply, so there is no count to reconcile and two
/// markets of one line may differ in all of it.
#[test]
fn a_markets_ladder_is_its_own_bounds_and_rates() {
    let line = line_record(&tiered());
    let content = market_content(
        &line,
        &MoneyView {
            tiers: Some(ladder([500, 400])),
            ..MoneyView::default()
        },
        &MarketPolicyView {
            tax_inclusive: Some(true),
            ..MarketPolicyView::default()
        },
    )
    .expect("a whole ladder converts");

    let bounds: Vec<(u64, BandTop)> = content
        .row
        .bands
        .iter()
        .map(|band| (band.from_qty, band.to_qty))
        .collect();
    assert_eq!(
        bounds,
        vec![(0, BandTop::Closed(100)), (100, BandTop::Open)]
    );
    let rates: Vec<i64> = content
        .row
        .bands
        .iter()
        .map(|band| band.unit_price_rate.nano_minor())
        .collect();
    assert_eq!(rates, vec![500, 400]);
    assert!(
        content.tax_inclusive,
        "and the market's policy travels with it"
    );

    // A sibling market of the same line, shaped differently, converts as well.
    let sibling = market_content(
        &line,
        &MoneyView {
            tiers: Some(vec![
                TierView {
                    from_qty: 0,
                    to_qty: Some(50),
                    rate_nano_minor: 600,
                },
                TierView {
                    from_qty: 50,
                    to_qty: Some(500),
                    rate_nano_minor: 450,
                },
                TierView {
                    from_qty: 500,
                    to_qty: None,
                    rate_nano_minor: 300,
                },
            ]),
            ..MoneyView::default()
        },
        &MarketPolicyView::default(),
    )
    .expect("a differently shaped ladder converts under the same line");
    assert_eq!(sibling.row.bands.len(), 3);
    assert_eq!(sibling.row.model_kind, content.row.model_kind);
}

/// A rate outside its scale is refused naming the ladder's own member.
#[test]
fn a_negative_band_rate_is_refused_by_name() {
    let line = line_record(&tiered());
    let refused = market_content(
        &line,
        &MoneyView {
            tiers: Some(ladder([500, -1])),
            ..MoneyView::default()
        },
        &MarketPolicyView::default(),
    );
    let Err(DomainError::InvalidRequest(detail)) = refused else {
        panic!("a negative rate must be refused: {refused:?}");
    };
    assert!(
        detail.contains("money.tiers.rate_nano_minor"),
        "naming the member: {detail}"
    );
}

/// The round trip a client sees: a ladder written is the ladder read.
#[test]
fn a_ladder_reads_back_as_it_was_written() {
    let line = line_record(&tiered());
    let content = market_content(
        &line,
        &MoneyView {
            tiers: Some(ladder([500, 400])),
            ..MoneyView::default()
        },
        &MarketPolicyView::default(),
    )
    .expect("a whole ladder converts");
    let (_, money) = crate::domain::market_price::split_row(content.row);
    let view = money_view_of(&money);
    let read: Vec<(u64, Option<u64>, i64)> = view
        .tiers
        .expect("a tiered market renders its ladder")
        .iter()
        .map(|tier| (tier.from_qty, tier.to_qty, tier.rate_nano_minor))
        .collect();
    assert_eq!(read, vec![(0, Some(100), 500), (100, None, 400)]);
}

/// **The shared half of a market's content is the line's, untouched.** The market
/// door submits a whole `PriceContent`, and every member of it that is not money
/// has to be what the version already stores — otherwise a monetary edit would
/// carry a stale structure back over the line.
#[test]
fn a_markets_content_carries_the_lines_structure_verbatim() {
    let line = line_record(&StructureView {
        model_kind: Some("per_unit".to_owned()),
        billing_granularity: Some("per_hour".to_owned()),
        min_qty_usage: Some(5),
        invoice_line_template: Some("{sku}".to_owned()),
        ..StructureView::default()
    });
    let content = market_content(
        &line,
        &MoneyView {
            unit_rate_nano_minor: Some(1_000),
            ..MoneyView::default()
        },
        &MarketPolicyView::default(),
    )
    .expect("the money converts");

    assert_eq!(content.row.model_kind, line.content.row.model_kind);
    assert_eq!(
        content.row.billing_granularity,
        line.content.row.billing_granularity
    );
    assert_eq!(content.row.min_qty_usage, Some(5));
    assert_eq!(
        content.row.invoice_line_template.as_deref(),
        Some("{sku}"),
        "the descriptor is the line's"
    );
    assert_eq!(
        content.row.unit_rate.map(RateMinor::nano_minor),
        Some(1_000),
        "and only the money is the market's"
    );
}
