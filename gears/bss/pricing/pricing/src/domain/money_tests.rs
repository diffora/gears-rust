//! Tests for the ISO 4217 money primitives.

use super::{
    CurrencyCode, DEFAULT_MINOR_UNIT, MinorAmount, check_decimal, check_scale, fraction_digits,
    is_expressible,
};
use crate::domain::error::DomainError;

fn code(raw: &str) -> CurrencyCode {
    CurrencyCode::new(raw).expect("test currency code is well-formed")
}

#[test]
fn one_currency_has_one_spelling() {
    // `currency` is a scope-key axis: if `usd` and `USD` were two values, two
    // rows on the same key would both pass the duplicate-key index.
    assert_eq!(code("usd"), code("USD"));
    assert_eq!(code(" uSd ").as_str(), "USD");
}

#[test]
fn a_code_that_is_not_three_letters_is_refused() {
    for raw in ["US", "USDX", "US1", "", "U$D"] {
        assert!(
            matches!(CurrencyCode::new(raw), Err(DomainError::CurrencyInvalid(_))),
            "expected {raw} to be refused"
        );
    }
}

#[test]
fn the_zero_decimal_currencies_take_no_minor_unit() {
    assert_eq!(code("JPY").minor_unit(), 0);
    assert_eq!(code("KRW").minor_unit(), 0);
    assert_eq!(code("ISK").minor_unit(), 0);
}

#[test]
fn the_three_decimal_currencies_take_three() {
    assert_eq!(code("BHD").minor_unit(), 3);
    assert_eq!(code("KWD").minor_unit(), 3);
    assert_eq!(code("OMR").minor_unit(), 3);
}

#[test]
fn a_code_outside_the_launch_subset_falls_back_to_two() {
    // The table is honest about being partial, so the fallback is the
    // behaviour that actually ships for most codes and is tested as such.
    assert_eq!(code("USD").minor_unit(), DEFAULT_MINOR_UNIT);
    assert_eq!(code("ZWG").minor_unit(), DEFAULT_MINOR_UNIT);
}

#[test]
fn jpy_rejects_two_decimals_while_bhd_accepts_three() {
    // The whole reason `PRECISION_EXCEEDED` exists: there is no flat
    // two-decimal rule, so a fixed scale would be wrong in both directions.
    assert!(check_decimal(&code("JPY"), "100.00").is_err());
    assert!(check_decimal(&code("BHD"), "1.234").is_ok());
    assert!(check_decimal(&code("JPY"), "100").is_ok());
    assert!(check_decimal(&code("BHD"), "1.2345").is_err());
}

#[test]
fn precision_is_judged_on_the_declared_scale_not_the_value() {
    // `100.00` on JPY is worth exactly 100 yen, and is still refused: the
    // author has declared a two-decimal price on a currency with no decimals,
    // and freezing that into the read model would leave every consumer to
    // re-derive which digits were real.
    assert!(!is_expressible(&code("JPY"), 2));
    assert!(matches!(
        check_scale(&code("JPY"), 2),
        Err(DomainError::PrecisionExceeded(_))
    ));
}

#[test]
fn a_scale_below_the_minor_unit_is_expressible() {
    // Under-declaring is not an error: USD 5 is a lawful price.
    assert!(is_expressible(&code("USD"), 0));
    assert!(check_decimal(&code("USD"), "5").is_ok());
}

#[test]
fn fraction_digits_counts_what_the_author_wrote() {
    assert_eq!(fraction_digits("100").expect("integer literal"), 0);
    assert_eq!(fraction_digits("100.5").expect("one decimal"), 1);
    assert_eq!(fraction_digits("-100.50").expect("signed literal"), 2);
}

#[test]
fn a_malformed_literal_is_a_bad_request_not_a_precision_failure() {
    // Two different problems reach the author under two different codes; a
    // caller that typed `1.2.3` is not told its currency has too few decimals.
    for raw in ["1.2.3", "abc", "", "1.", ".5", "1..2"] {
        assert!(
            matches!(fraction_digits(raw), Err(DomainError::InvalidRequest(_))),
            "expected {raw} to be malformed"
        );
    }
}

#[test]
fn a_negative_amount_is_refused() {
    // Typed credit rows are Future scope, so a negative price is a mistake
    // caught at the boundary rather than an unsupported feature carried inward.
    assert!(matches!(
        MinorAmount::new(-1),
        Err(DomainError::AmountNegative(_))
    ));
}

#[test]
fn zero_is_a_valid_amount() {
    // The rule is non-negative, not positive: a free trial phase and a
    // zero-rated usage row are both authored as `0`.
    assert_eq!(MinorAmount::new(0).expect("zero is valid").get(), 0);
}

#[test]
fn a_sign_error_is_not_reported_as_a_precision_error() {
    // `fraction_digits` accepts the sign so that `-1.005` on USD is answered by
    // the amount rule, not by the scale rule; the author gets one fault at a
    // time and the two codes stay distinguishable.
    assert_eq!(fraction_digits("-1.00").expect("signed literal"), 2);
    assert!(check_decimal(&code("USD"), "-1.00").is_ok());
    assert!(MinorAmount::new(-100).is_err());
}
