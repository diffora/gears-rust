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
        tiers: Some(vec![
            TierView {
                from_qty: 0,
                to_qty: Some(100),
            },
            TierView {
                from_qty: 100,
                to_qty: None,
            },
        ]),
        ..StructureView::default()
    }
}

/// A line record standing in for one the repository would have read.
fn line_record(structure: &StructureView) -> LineRecord {
    let key = line_key_of(plan(), &key_request("usage")).expect("the axes parse");
    let content = structure_content(&key, structure).expect("the structure converts");
    let tiers = content
        .row
        .bands
        .iter()
        .map(|band| TierGeometry {
            from_qty: band.from_qty,
            to_qty: band.to_qty,
        })
        .collect();
    LineRecord {
        charge_line_id: Uuid::from_u128(0x11e0),
        line_version_id: Uuid::from_u128(0x11e2),
        plan_revision: 0,
        lifecycle_state: LifecycleState::Draft,
        row_version: RowVersion::new(0),
        scope_key: key,
        content,
        tiers,
        resolved_invoice_line_template: None,
        resolved_gl_code: None,
        created_by: Uuid::nil(),
        created_at_utc: OffsetDateTime::UNIX_EPOCH,
    }
}

/// **A structure carries no money, whatever it is converted through.**
///
/// The conversion renders a structure as the flat content it is the shared half of
/// so that one reader parses every token — and the zero rates that rendering needs
/// are an artefact of it. A structure that arrived carrying an amount would be
/// indistinguishable from one that did not, which is the confusion this pins.
#[test]
fn a_converted_structure_has_geometry_and_no_money() {
    let key = line_key_of(plan(), &key_request("usage")).expect("the axes parse");
    let content = structure_content(&key, &tiered()).expect("the structure converts");

    assert_eq!(content.row.bands.len(), 2, "the geometry is carried");
    assert_eq!(content.row.bands[0].from_qty, 0);
    assert_eq!(content.row.bands[1].to_qty, BandTop::Open);
    assert!(content.row.amount_minor.is_none());
    assert!(content.row.unit_rate.is_none());
    assert!(content.row.package_price_minor.is_none());
    assert!(content.row.reserved_rate.is_none());
    assert!(!content.tax_inclusive);
    assert!(content.tax_category_ref.is_none());
    assert!(content.rounding_policy_ref.is_none());
    assert!(content.grandfather_until.is_none());
    assert!(
        content
            .row
            .bands
            .iter()
            .all(|band| band.unit_price_rate.nano_minor() == 0),
        "and the rates the rendering needs are zero, never a price"
    );
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

/// **Rates fill the line's geometry by position, and a count that cannot be
/// positioned is refused.** A surplus rate has no band to be the rate of, and a
/// short ladder would bill the uncovered tiers at nothing.
#[test]
fn a_rate_count_that_is_not_the_tier_count_is_refused() {
    let line = line_record(&tiered());

    for rates in [vec![500_i64], vec![500, 400, 300]] {
        let refused = market_content(
            &line,
            &MoneyView {
                tier_rates_nano_minor: Some(rates.clone()),
                ..MoneyView::default()
            },
            &MarketPolicyView::default(),
        );
        let Err(DomainError::ValidationFailed(report)) = refused else {
            panic!("{} rate(s) against 2 tiers must be refused", rates.len());
        };
        assert_eq!(
            report.violations[0].code, "MARKET_TIER_RATE_COUNT_MISMATCH",
            "and it is the ladder's own code: {report:?}"
        );
        assert_eq!(
            report.violations[0].subject, "money.tier_rates_nano_minor",
            "naming the member that cannot be positioned: {report:?}"
        );
    }
}

/// No rates at all is an unfinished draft, which a draft is allowed to be.
#[test]
fn a_market_with_no_rates_yet_converts() {
    let line = line_record(&tiered());
    let content = market_content(&line, &MoneyView::default(), &MarketPolicyView::default())
        .expect("an unpriced market is a legal draft");
    assert!(content.row.bands.is_empty(), "no rates, so no bands");
}

/// The join is by position, and the geometry is the line's rather than the
/// request's — a market cannot move a bound by sending rates.
#[test]
fn rates_join_the_lines_geometry_in_its_own_order() {
    let line = line_record(&tiered());
    let content = market_content(
        &line,
        &MoneyView {
            tier_rates_nano_minor: Some(vec![500, 400]),
            ..MoneyView::default()
        },
        &MarketPolicyView {
            tax_inclusive: Some(true),
            ..MarketPolicyView::default()
        },
    )
    .expect("two rates against two tiers");

    let bounds: Vec<u64> = content.row.bands.iter().map(|band| band.from_qty).collect();
    assert_eq!(bounds, vec![0, 100], "the line's bounds, not the market's");
    let rates: Vec<i64> = content
        .row
        .bands
        .iter()
        .map(|band| band.unit_price_rate.nano_minor())
        .collect();
    assert_eq!(rates, vec![500, 400], "in the line's quantity order");
    assert!(
        content.tax_inclusive,
        "and the market's policy travels with it"
    );
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
