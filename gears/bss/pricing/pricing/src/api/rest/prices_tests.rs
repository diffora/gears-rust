//! The price views' wire shape, and the two distinctions a collapsing
//! serializer would destroy.
//!
//! Everything needing a store or a PDP is in `tests/rest_prices.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{PriceRowView, TierBandView, band_of};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{BandTop, PriceRow, TierBand};
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
