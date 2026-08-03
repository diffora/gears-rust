//! The price views' wire shape, and the two distinctions a collapsing
//! serializer would destroy.
//!
//! Everything needing a store or a PDP is in `tests/rest_prices.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{
    IncludedAllowanceView, PriceContentView, PriceRowView, TierBandView, band_of, content_of,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{
    BandTop, IncludedAllowance, PriceRow, RolloverPolicy, TierBand, TierQualificationWindow,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

fn record(bands: Vec<TierBand>) -> PriceRecord {
    let key = ScopeKey::new(
        PlanId::new(Uuid::from_u128(0x91a4)),
        CurrencyCode::new("USD").expect("currency"),
        Region::new("EU").expect("region"),
        PhaseId::new(Uuid::from_u128(0x9ba5e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
        Cohort::None,
    )
    .expect("key");
    PriceRecord {
        price_id: Uuid::from_u128(0x9_71ce),
        scope_key: key,
        row: PriceRow {
            bands,
            ..PriceRow::new(ChargeKind::Usage, None)
        },
        tax_inclusive: false,
        billing_timing: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac70),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap(),
        row_version: RowVersion::new(2),
    }
}

fn body(view: &PriceRowView) -> serde_json::Value {
    serde_json::to_value(view).expect("the view serializes")
}

#[test]
fn an_open_top_band_is_a_null_upper_bound_and_round_trips_as_one() {
    // `BandTop` is an enum rather than an `Option<u64>` precisely because "open"
    // is a STATE of the band; the wire spells it `null`, and a round trip that
    // read `null` as "no bound given" would silently close the top band D-17
    // requires to be open.
    let open = TierBand::open(0, MinorAmount::new(500).expect("amount"));
    let rendered = body(&PriceRowView::from(&record(vec![open])));
    assert!(
        rendered["content"]["bands"][0]["to_qty"].is_null(),
        "{rendered}"
    );

    let parsed = band_of(&TierBandView {
        from_qty: 0,
        to_qty: None,
        unit_price_minor: 500,
    })
    .expect("an open band parses");
    assert_eq!(parsed.to_qty, BandTop::Open);
}

#[test]
fn a_closed_band_carries_its_exclusive_upper_bound() {
    let closed = TierBand::closed(0, 100, MinorAmount::new(500).expect("amount"));
    let rendered = body(&PriceRowView::from(&record(vec![closed])));
    assert_eq!(
        rendered["content"]["bands"][0]["to_qty"],
        serde_json::json!(100),
        "{rendered}"
    );
}

#[test]
fn a_negative_unit_price_is_refused_rather_than_stored() {
    // Typed credit rows are deliberately out of scope, so a negative price is a
    // mistake and not an unsupported feature.
    let refusal = band_of(&TierBandView {
        from_qty: 0,
        to_qty: None,
        unit_price_minor: -1,
    })
    .expect_err("a negative amount is refused");
    assert!(
        matches!(
            refusal,
            crate::domain::error::DomainError::AmountNegative(_)
        ),
        "{refusal:?}"
    );
}

/// A view with everything a well-formed `flat` row needs and nothing more.
fn clean_view() -> PriceContentView {
    PriceContentView {
        model_kind: Some("flat".to_owned()),
        amount_minor: Some(1_500),
        bands: None,
        package_size: None,
        package_price_minor: None,
        quantity_source: None,
        manual_quantity: None,
        meter: None,
        dimension_key: None,
        billing_granularity: None,
        tier_aggregation_window: None,
        tier_qualification_window: None,
        aggregation_function: None,
        aggregation_granularity: None,
        max_hold_granules: None,
        included_allowance: None,
        tax_inclusive: Some(false),
        billing_timing: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

#[test]
fn the_read_half_still_renders_both_slice_ten_primitives() {
    // The request half refuses them; the RESPONSE half must not drop them. The
    // domain model, the storage round trip and the D-129 supersession guard all
    // carry both fields, so a read that omitted them would lose a field that
    // guard compares between a predecessor and its successor - and a row can
    // hold either value from before the refusal, or from a path that is not this
    // surface.
    let mut stored = record(Vec::new());
    stored.row.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);
    stored.row.included_allowance = Some(IncludedAllowance {
        quantity: 50,
        rollover_policy: RolloverPolicy::Carry,
    });

    let rendered = body(&PriceRowView::from(&stored));

    assert_eq!(
        rendered["content"]["tier_qualification_window"],
        serde_json::json!("trailing_period"),
        "{rendered}"
    );
    assert_eq!(
        rendered["content"]["included_allowance"]["quantity"],
        serde_json::json!(50),
        "{rendered}"
    );
    assert_eq!(
        rendered["content"]["included_allowance"]["rollover_policy"],
        serde_json::json!("carry"),
        "{rendered}"
    );
}

#[test]
fn the_request_half_refuses_each_slice_ten_primitive_and_accepts_their_absence() {
    // Delete `refuse_unlanded_primitives`' call in `content_of` and the two
    // refusal arms below stop failing - as does
    // `rest_prices.rs::a_create_carrying_a_tier_qualification_window_is_refused_at_any_value`.
    content_of(&clean_view()).expect("a row carrying neither primitive still converts");

    for window in ["trailing_period", "current"] {
        let refusal = content_of(&PriceContentView {
            tier_qualification_window: Some(window.to_owned()),
            ..clean_view()
        })
        .expect_err("an explicit window of ANY value is refused");
        assert!(
            matches!(
                &refusal,
                crate::domain::error::DomainError::InvalidRequest(detail)
                    if detail.contains("tier_qualification_window")
            ),
            "{refusal:?}"
        );
    }

    for policy in ["none", "carry"] {
        let refusal = content_of(&PriceContentView {
            included_allowance: Some(IncludedAllowanceView {
                quantity: 100,
                rollover_policy: policy.to_owned(),
            }),
            ..clean_view()
        })
        .expect_err("an allowance under either policy is refused");
        assert!(
            matches!(
                &refusal,
                crate::domain::error::DomainError::InvalidRequest(detail)
                    if detail.contains("included_allowance")
            ),
            "{refusal:?}"
        );
    }
}

#[test]
fn the_refusal_precedes_the_enum_spelling_check() {
    // Otherwise a caller who misspells the window is told the token is wrong and
    // infers the field is supported - and a caller who spells it right is told
    // nothing at all. The refusal is about the FIELD, so it runs first.
    let refusal = content_of(&PriceContentView {
        tier_qualification_window: Some("not_a_window".to_owned()),
        ..clean_view()
    })
    .expect_err("a misspelled window is refused too");
    let crate::domain::error::DomainError::InvalidRequest(detail) = &refusal else {
        panic!("{refusal:?}");
    };
    assert!(
        detail.contains("is not supported yet"),
        "the field's absence from the gear is the reason, not the token: {detail}"
    );
}

#[test]
fn the_view_names_the_rows_own_version_and_its_whole_key() {
    // D-141: the token is the price row's OWN version column, never derived from
    // the plan's - a per-row bulk conflict means nothing if every row of a plan
    // shares one version.
    let rendered = body(&PriceRowView::from(&record(Vec::new())));

    assert_eq!(rendered["row_version"], serde_json::json!(2));
    assert_eq!(
        rendered["scope_key"]["price_overlay"],
        serde_json::json!("base")
    );
    assert_eq!(
        rendered["scope_key"]["charge_kind"],
        serde_json::json!("usage")
    );
    assert!(rendered["scope_key"]["cohort"].is_null(), "{rendered}");
}
