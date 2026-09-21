//! Which market price a buyer in a region is sold: **the region's own price if
//! it has one, else the currency's.**
//!
//! Prices are authored per currency and apply to every region; a region carries
//! a price of its own only where it needs one. The currency-wide price is filed
//! under the reserved region [`CURRENCY_WIDE_REGION`] — `global`, the value every
//! tenant's region taxonomy starts with (D-354). Region stays an axis of the
//! market key: an override needs its own versions, windows, supersession and
//! conflict checks, and every one of those is keyed on the full market key.
//!
//! # The whole rule
//!
//! ```text
//! price at instant T for (overlay O, currency C, region R) =
//!     the covering window on (O,    C, R)
//!     else                   (O,    C, global)
//!     else                   (base, C, R)
//!     else                   (base, C, global)
//!     else refused — no FX fallback and no cross-currency fallback
//! ```
//!
//! **An overlay outranks a region**, so the overlay is the outer loop of
//! [`resolution_order`] and the region the inner one.
//!
//! **What is exercisable in this gear is the inner loop.**
//! [`PriceOverlay`](crate::domain::scope_key::PriceOverlay) has one variant,
//! `Base`: every price row this gear authors is a base row, and a partner or
//! brand overlay is a separate object whose amounts are keyed on currency alone
//! and which the consumer composes at evaluation. So the first two steps name
//! keys nothing here can file, and the order that ranks them is a statement of
//! the consumer contract rather than a branch this gear reaches. It is written
//! out anyway — deduplicated, so today it is `R` then `global` — because a second
//! reading of this order, added the day a second variant arrives, is exactly the
//! defect the rule exists to prevent.
//!
//! # Which region this is
//!
//! The **commercial territory of the buyer** — the `region` axis of
//! [`MarketPriceScopeKey`]. It is not where a resource runs: that is a usage
//! dimension (`dimension_key`), a separate line, and nothing here touches it.
//!
//! # An override is a whole row
//!
//! Resolution answers a **key**. Nothing of the `global` row is inherited by an
//! override — not an amount, not a band, not the tax display — so there is no
//! field-level effective value to compute, and a consumer that pinned the
//! `priceId` it resolved has pinned everything.
//!
//! # One function
//!
//! Preview, the sellability surface, the completeness rules and the bundle plane
//! call [`resolve`] and [`resolves_statically`]. A second implementation of this
//! order is a finding.

use crate::domain::money::CurrencyCode;
use crate::domain::read_model::GLOBAL_SCOPE;
use crate::domain::scope_key::{ChargeLineScopeKey, MarketPriceScopeKey, PriceOverlay, Region};

/// The reserved region a currency-wide price is filed under.
pub const CURRENCY_WIDE_REGION: &str = GLOBAL_SCOPE;

/// Is `region` the reserved currency-wide one?
#[must_use]
pub fn is_currency_wide(region: &Region) -> bool {
    region.as_str() == CURRENCY_WIDE_REGION
}

/// The `(overlay, region)` pairs tried for a purchase on `(overlay, region)`, in
/// order, each at most once.
///
/// Four steps when the overlay is not the base and the region is not `global`;
/// fewer where two of them name the same key, which is every case this gear can
/// reach today (see the module doc).
#[must_use]
pub fn resolution_order(overlay: PriceOverlay, region: &Region) -> Vec<(PriceOverlay, Region)> {
    let mut overlays = vec![overlay];
    if overlay != PriceOverlay::Base {
        overlays.push(PriceOverlay::Base);
    }
    let mut regions = vec![region.clone()];
    if !is_currency_wide(region)
        && let Ok(global) = Region::new(CURRENCY_WIDE_REGION)
    {
        regions.push(global);
    }
    overlays
        .into_iter()
        .flat_map(|overlay| regions.iter().map(move |region| (overlay, region.clone())))
        .collect()
}

/// The market key of `charge` that prices `(overlay, currency, region)`, among
/// `candidates`, taking the first step of [`resolution_order`] that `admits`.
///
/// `charge` is compared on every logical axis but the overlay, so an overlay's
/// price and the base price of one charge are candidates for the one purchase.
/// `admits` is the caller's question of a key — "does a window cover `T`" for a
/// purchase at an instant, "is it priced at all" for a static rule — and is
/// asked of each step in turn, which is what makes an override whose window has
/// ended fall back rather than refuse.
pub fn resolve<'a>(
    candidates: impl IntoIterator<Item = &'a MarketPriceScopeKey>,
    charge: &ChargeLineScopeKey,
    currency: &CurrencyCode,
    region: &Region,
    admits: impl Fn(&MarketPriceScopeKey) -> bool,
) -> Option<&'a MarketPriceScopeKey> {
    let of_this_charge: Vec<&MarketPriceScopeKey> = candidates
        .into_iter()
        .filter(|key| key.currency() == currency && key.line().is_same_charge_as(charge))
        .collect();
    resolution_order(charge.price_overlay(), region)
        .into_iter()
        .find_map(|(overlay, region)| {
            of_this_charge.iter().copied().find(|key| {
                key.price_overlay() == overlay && *key.region() == region && admits(key)
            })
        })
}

/// Is `(currency, region)` a market `charge` can be bought in at all — by its
/// own row or by the currency-wide one?
///
/// [`resolve`] with no instant: the question the completeness rules ask, which
/// read what is *authored* and not what is *scheduled*.
#[must_use]
pub fn resolves_statically<'a>(
    candidates: impl IntoIterator<Item = &'a MarketPriceScopeKey>,
    charge: &ChargeLineScopeKey,
    currency: &CurrencyCode,
    region: &Region,
) -> bool {
    resolve(candidates, charge, currency, region, |_| true).is_some()
}

/// The markets every line of a plan **owes a price in**, given the markets its
/// candidate lines sell on.
///
/// Per currency, and on one of two footings:
///
/// - **some line sells the currency everywhere** — a `global` price exists in it.
///   Then the plan sells that currency in every region, so every line owes the
///   `global` price and nothing else. An override on top obliges no sibling: a
///   line without one falls back. Without this arm a buyer in `FR` would resolve
///   the line that has a `global` price and not the one that has only a `DE` row.
/// - **no line does** — the rule as it always was, per pair: every line owes
///   every region some line sells the currency in.
///
/// A plan declares no set of regions it sells into; sold markets are *derived*
/// from the rows' keys, and so is this.
#[must_use]
pub fn owed_markets(
    sold: &std::collections::BTreeSet<(CurrencyCode, Region)>,
) -> std::collections::BTreeSet<(CurrencyCode, Region)> {
    let everywhere: std::collections::BTreeSet<&CurrencyCode> = sold
        .iter()
        .filter(|(_, region)| is_currency_wide(region))
        .map(|(currency, _)| currency)
        .collect();
    sold.iter()
        .filter(|(currency, region)| !everywhere.contains(currency) || is_currency_wide(region))
        .cloned()
        .collect()
}

/// Is a bound, a rule or an add-on stated on `(currency, region)` stated on a
/// market the plan sells — by that pair's own row, or by the currency-wide one?
#[must_use]
pub fn is_sold(
    sold: &std::collections::BTreeSet<(CurrencyCode, Region)>,
    currency: &CurrencyCode,
    region: &Region,
) -> bool {
    sold.iter().any(|(sold_currency, sold_region)| {
        sold_currency == currency && (sold_region == region || is_currency_wide(sold_region))
    })
}

#[cfg(test)]
#[path = "market_resolution_tests.rs"]
mod market_resolution_tests;
