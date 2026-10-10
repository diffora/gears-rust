//! Integer minor units for the parked `v1` event and alarm payloads.
//!
//! The `v1` payloads still carry `*_minor: i64` fields. A posted amount can exceed
//! what `i64` minor units describe (the scale goes up to 28; at scale 18, anything
//! above about 9.22 units overflows), so every emitter goes through one policy:
//! exact when the value fits, otherwise saturated by sign with a warning that
//! carries the exact amount. The payloads are informational, so an amount they
//! cannot describe never fails a posting, rolls back a run or drops an alarm.

use bss_ledger_sdk::{PostedMoney, canonical_decimal};

/// Integer minor units of `money` for the parked `v1` payload field `field`.
///
/// Exact whenever the value fits `i64`; otherwise `i64::MAX` for a positive and
/// `i64::MIN` for a negative amount, logged at `warn` with the exact decimal.
#[must_use]
pub(crate) fn v1_minor_units(money: &PostedMoney, field: &'static str) -> i64 {
    crate::domain::money::minor_units(money).unwrap_or_else(|| {
        tracing::warn!(
            target: "bss-ledger",
            field,
            amount = %canonical_decimal(money.amount()),
            currency = money.currency().code(),
            currency_scale = money.currency().scale(),
            "bss-ledger: amount saturated in a parked v1 payload field"
        );
        if money.amount().is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

#[cfg(test)]
#[path = "v1_payload_tests.rs"]
mod tests;
