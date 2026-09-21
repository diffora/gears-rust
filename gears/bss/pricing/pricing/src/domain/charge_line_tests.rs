//! Tests for shared charge-line structure and line-version identity.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{ChargeLineVersion, ChargeStructure};
use crate::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use crate::domain::money::CurrencyCode;
use crate::domain::price_row::ModelKind;
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    Region, SkuId,
};
use uuid::Uuid;

fn line() -> ChargeLineScopeKey {
    ChargeLineScopeKey::new(
        PlanId::new(Uuid::from_u128(0x11)),
        PhaseId::new(Uuid::from_u128(0x22)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
        SkuId::new(Uuid::from_u128(5)),
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn timing() -> ProrationContract {
    ProrationContract {
        billing_anchor_policy: BillingAnchorPolicy::SubscriptionStart,
        proration_basis: ProrationBasis::CalendarDaysActual,
        credit_on_downgrade: false,
    }
}

#[test]
fn billing_timing_is_authored_on_the_line_version_not_per_currency() {
    let version = ChargeLineVersion {
        charge_line_id: Uuid::from_u128(0xa1),
        line_version_id: Uuid::from_u128(0xa2),
        scope_key: line(),
        structure: ChargeStructure::new(ChargeKind::Recurring, Some(ModelKind::Flat)),
        billing_timing: Some("advance".to_owned()),
        proration_contract: Some(timing()),
    };
    let us = MarketPriceScopeKey::new(
        version.scope_key.clone(),
        CurrencyCode::new("usd").expect("USD is three letters"),
        Region::new("US").expect("a non-blank region"),
    );
    let eur = MarketPriceScopeKey::new(
        version.scope_key.clone(),
        CurrencyCode::new("eur").expect("EUR is three letters"),
        Region::new("DE").expect("a non-blank region"),
    );

    assert_eq!(us.line(), eur.line());
    assert_ne!(us, eur);
    assert_eq!(version.billing_timing.as_deref(), Some("advance"));
    assert_eq!(version.proration_contract, Some(timing()));
    assert_eq!(us.line(), &version.scope_key);
}

#[test]
fn an_empty_structure_matches_an_empty_price_row_on_non_money_fields() {
    let structure = ChargeStructure::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    assert_eq!(structure.dimension_key, "");
    assert_eq!(structure.sku_id, SkuId::new(Uuid::nil()));
    assert_eq!(structure.subject(), "usage/graduated");
}
