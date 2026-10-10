//! Bounded exact fractions in major units, narrowed only at an explicit posting boundary.

use std::cmp::Ordering;
use std::sync::LazyLock;

use bigdecimal::num_bigint::{BigInt, BigUint};
use bigdecimal::num_traits::{One, Signed, ToPrimitive, Zero};
use rust_decimal::Decimal;

use crate::{CurrencySpec, MoneyError, PostedMoney};

const FINAL_DIGITS: u32 = 256;
const PRODUCT_DIGITS: u32 = 512;
const SUM_DIGITS: u32 = 513;

/// Exclusive magnitude bounds: an integer has more than `N` decimal digits
/// exactly when its magnitude is at least `10^N`. Computed once, so a digit
/// check is one comparison instead of a clone and a base-10 rendering.
static FINAL_BOUND: LazyLock<BigUint> = LazyLock::new(|| digit_bound(FINAL_DIGITS));
static PRODUCT_BOUND: LazyLock<BigUint> = LazyLock::new(|| digit_bound(PRODUCT_DIGITS));
static SUM_BOUND: LazyLock<BigUint> = LazyLock::new(|| digit_bound(SUM_DIGITS));

/// An exact intermediate amount, with a positive, reduced denominator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactAmount {
    numerator: BigInt,
    denominator: BigInt,
}

/// A validation or resource-limit failure in exact monetary arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExactError {
    /// The final posting or its metadata violates the money contract.
    #[error(transparent)]
    Money(#[from] MoneyError),
    /// A reduced integer exceeds 256 digits or scratch exceeds its derived bound.
    #[error("exact arithmetic limit exceeded")]
    ArithmeticLimit,
    /// The divisor is zero.
    #[error("division by zero")]
    DivisionByZero,
    /// Allocation requires at least one weight.
    #[error("allocation weights must be non-empty")]
    EmptyWeights,
    /// Allocation weights cannot be negative.
    #[error("allocation weights must be non-negative")]
    NegativeWeight,
    /// The operation has more terms than its business limit allows.
    #[error("too many terms")]
    TooManyTerms,
}

impl ExactAmount {
    /// Preserve all decimal digits as a reduced fraction; every Decimal fits the budget.
    #[must_use]
    pub fn from_decimal(value: Decimal) -> Self {
        Self::normalize(BigInt::from(value.mantissa()), ten_pow(value.scale()))
    }

    /// Inspect the exact sign without narrowing an intermediate to a posted amount.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.numerator.is_negative()
    }

    /// Add exactly, allowing bounded unreduced intermediates to cancel.
    ///
    /// # Errors
    /// Returns [`ExactError::ArithmeticLimit`] if the reduced result exceeds the budget.
    pub fn checked_add(&self, rhs: &Self) -> Result<Self, ExactError> {
        self.combine(rhs, false)
    }

    /// Subtract exactly, allowing bounded unreduced intermediates to cancel.
    ///
    /// # Errors
    /// Returns [`ExactError::ArithmeticLimit`] if the reduced result exceeds the budget.
    pub fn checked_sub(&self, rhs: &Self) -> Result<Self, ExactError> {
        self.combine(rhs, true)
    }

    /// Multiply without rounding or narrowing to the decimal carrier.
    ///
    /// # Errors
    /// Returns [`ExactError::ArithmeticLimit`] if the reduced result exceeds the budget.
    pub fn checked_mul(&self, rhs: &Self) -> Result<Self, ExactError> {
        self.check_operands(rhs)?;
        Self::reduce(
            product(&self.numerator, &rhs.numerator)?,
            product(&self.denominator, &rhs.denominator)?,
        )
    }

    /// Divide without approximating a non-terminating quotient.
    ///
    /// # Errors
    /// Returns [`ExactError::DivisionByZero`] for zero divisors, or
    /// [`ExactError::ArithmeticLimit`] if the reduced result exceeds the budget.
    pub fn checked_div(&self, rhs: &Self) -> Result<Self, ExactError> {
        if rhs.numerator.is_zero() {
            return Err(ExactError::DivisionByZero);
        }
        self.check_operands(rhs)?;
        Self::reduce(
            product(&self.numerator, &rhs.denominator)?,
            product(&self.denominator, &rhs.numerator)?,
        )
    }

    /// Encode an exact posting at the supplied currency scale without rounding.
    ///
    /// # Errors
    /// Returns a money error for a fractional posting increment or final overflow,
    /// or [`ExactError::ArithmeticLimit`] if the fraction exceeds its budget.
    pub fn into_posted_exact(self, currency: CurrencySpec) -> Result<PostedMoney, ExactError> {
        self.check_budget()?;
        let scaled = product(&self.numerator, &ten_pow(u32::from(currency.scale())))?;
        if !(&scaled % &self.denominator).is_zero() {
            return Err(MoneyError::InvalidPostingIncrement.into());
        }
        posted(scaled / &self.denominator, currency)
    }

    /// Render exact major-unit evidence at a known posting grid without imposing
    /// the bounded `PostedMoney` coefficient limit on an intermediate.
    ///
    /// # Errors
    /// Rejects invalid scale, fractional increments and the exact arithmetic budget.
    pub fn canonical_at_scale(&self, scale: u8) -> Result<String, ExactError> {
        if scale > 28 {
            return Err(MoneyError::ScaleOutOfRange.into());
        }
        self.check_budget()?;
        let scaled = product(&self.numerator, &ten_pow(u32::from(scale)))?;
        if !(&scaled % &self.denominator).is_zero() {
            return Err(MoneyError::InvalidPostingIncrement.into());
        }
        let coefficient = scaled / &self.denominator;
        let digits = coefficient.abs().to_str_radix(10);
        if coefficient.is_zero() {
            return Ok("0".to_owned());
        }
        let sign = if coefficient.is_negative() { "-" } else { "" };
        if scale == 0 {
            return Ok(format!("{sign}{digits}"));
        }
        let scale = usize::from(scale);
        let padded = if digits.len() <= scale {
            format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits)
        } else {
            digits
        };
        let (whole, fraction) = padded.split_at(padded.len() - scale);
        let fraction = fraction.trim_end_matches('0');
        if fraction.is_empty() {
            Ok(format!("{sign}{whole}"))
        } else {
            Ok(format!("{sign}{whole}.{fraction}"))
        }
    }

    /// Round once at the requested currency scale, with ties going to an even coefficient.
    ///
    /// # Errors
    /// Returns a money error for final overflow, or [`ExactError::ArithmeticLimit`]
    /// if the fraction exceeds its budget.
    pub fn round_half_even(self, currency: CurrencySpec) -> Result<PostedMoney, ExactError> {
        self.round_half_even_exact(currency.scale())?
            .into_posted_exact(currency)
    }

    /// Round to a posting increment while retaining an exact, unbounded money intermediate.
    /// Used when a later anchor residual may bring the rounded amount back into range.
    ///
    /// # Errors
    /// Returns a named scale or arithmetic-budget error. Final money bounds are checked
    /// only when the caller converts the completed posting with `into_posted_exact`.
    pub fn round_half_even_exact(self, scale: u8) -> Result<Self, ExactError> {
        if scale > 28 {
            return Err(MoneyError::ScaleOutOfRange.into());
        }
        self.check_budget()?;
        let scaled = product(&self.numerator.abs(), &ten_pow(u32::from(scale)))?;
        let mut quotient = &scaled / &self.denominator;
        let remainder = scaled % &self.denominator;
        let twice_remainder = product(&remainder, &BigInt::from(2))?;
        if twice_remainder > self.denominator
            || (twice_remainder == self.denominator && (&quotient % 2_u8) != BigInt::zero())
        {
            quotient += 1;
        }
        if self.numerator.is_negative() {
            quotient = -quotient;
        }
        Self::reduce(quotient, ten_pow(u32::from(scale)))
    }

    /// Add or subtract cross-products, whose sum can have at most 513 digits.
    fn combine(&self, rhs: &Self, subtract: bool) -> Result<Self, ExactError> {
        self.check_operands(rhs)?;
        let left = product(&self.numerator, &rhs.denominator)?;
        let right = product(&rhs.numerator, &self.denominator)?;
        let numerator = if subtract { left - right } else { left + right };
        check_digits(&numerator, &SUM_BOUND)?;
        Self::reduce(numerator, product(&self.denominator, &rhs.denominator)?)
    }

    /// Validate reduced inputs before allocating operation scratch.
    fn check_operands(&self, rhs: &Self) -> Result<(), ExactError> {
        self.check_budget()?;
        rhs.check_budget()
    }

    /// Enforce the reduced fraction's final resource budget.
    fn check_budget(&self) -> Result<(), ExactError> {
        check_digits(&self.numerator, &FINAL_BOUND)?;
        check_digits(&self.denominator, &FINAL_BOUND)
    }

    /// Validate scratch, normalize signs and common factors, then check final bounds.
    fn reduce(numerator: BigInt, denominator: BigInt) -> Result<Self, ExactError> {
        if denominator.is_zero() {
            return Err(ExactError::DivisionByZero);
        }
        check_digits(&numerator, &SUM_BOUND)?;
        check_digits(&denominator, &PRODUCT_BOUND)?;
        let result = Self::normalize(numerator, denominator);
        result.check_budget()?;
        Ok(result)
    }

    /// Reduce known bounded integers with a nonzero denominator using Euclid's algorithm.
    fn normalize(mut numerator: BigInt, mut denominator: BigInt) -> Self {
        if numerator.is_zero() {
            return Self {
                numerator: BigInt::zero(),
                denominator: BigInt::one(),
            };
        }
        if denominator.is_negative() {
            numerator = -numerator;
            denominator = -denominator;
        }
        let mut divisor = numerator.abs();
        let mut remainder = denominator.clone();
        while !remainder.is_zero() {
            let next = &divisor % &remainder;
            divisor = remainder;
            remainder = next;
        }
        Self {
            numerator: numerator / &divisor,
            denominator: denominator / divisor,
        }
    }
}

impl PartialOrd for ExactAmount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExactAmount {
    /// Exact value order. Denominators are positive and reduced, so comparing
    /// the cross products is exact and agrees with the derived `Eq`. The
    /// products of two in-budget fractions are bounded scratch, not a result.
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.numerator * &other.denominator).cmp(&(&other.numerator * &self.denominator))
    }
}

impl std::fmt::Display for ExactAmount {
    /// Decimal text when the fraction terminates within 28 places, otherwise
    /// `numerator/denominator`; for messages only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for scale in 0..=28_u8 {
            if let Ok(text) = self.canonical_at_scale(scale) {
                return f.write_str(&text);
            }
        }
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

/// Sum matching postings exactly and validate only the final decimal amount.
///
/// # Errors
/// Returns a money error for mismatched currency/scale or final overflow,
/// or [`ExactError::ArithmeticLimit`] when the exact arithmetic budget is exceeded.
pub fn sum_posted(
    values: &[PostedMoney],
    currency: CurrencySpec,
) -> Result<PostedMoney, ExactError> {
    // Validate all metadata before any addition.
    for value in values {
        value.currency().ensure_same(&currency)?;
    }
    let mut sum = ExactAmount::from_decimal(Decimal::ZERO);
    for value in values {
        sum = sum.checked_add(&ExactAmount::from_decimal(value.amount()))?;
    }
    sum.into_posted_exact(currency)
}

/// Return the exact positive encoding factor for a decimal scale.
fn ten_pow(scale: u32) -> BigInt {
    BigInt::from(10).pow(scale)
}

/// `10^digits`, the smallest magnitude with `digits + 1` decimal digits.
fn digit_bound(digits: u32) -> BigUint {
    BigUint::from(10_u8).pow(digits)
}

/// Check absolute decimal digits against a [`digit_bound`], counting zero as one
/// digit.
fn check_digits(value: &BigInt, bound: &BigUint) -> Result<(), ExactError> {
    if value.magnitude() >= bound {
        return Err(ExactError::ArithmeticLimit);
    }
    Ok(())
}

/// Multiply bounded operands and enforce the 512-digit scratch bound.
fn product(left: &BigInt, right: &BigInt) -> Result<BigInt, ExactError> {
    let value = left * right;
    check_digits(&value, &PRODUCT_BOUND)?;
    Ok(value)
}

/// Normalize an encoding coefficient before constructing the bounded decimal carrier.
fn posted(mut coefficient: BigInt, currency: CurrencySpec) -> Result<PostedMoney, ExactError> {
    let mut scale = u32::from(currency.scale());
    while scale > 0 && (&coefficient % 10_u8).is_zero() {
        coefficient /= 10_u8;
        scale -= 1;
    }
    if coefficient.abs() >= ten_pow(28) {
        return Err(MoneyError::AmountOutOfRange.into());
    }
    let coefficient = coefficient.to_i128().ok_or(MoneyError::AmountOutOfRange)?;
    Ok(PostedMoney::try_new(
        Decimal::from_i128_with_scale(coefficient, scale),
        currency,
    )?)
}

#[cfg(test)]
#[path = "exact_tests.rs"]
mod exact_tests;
