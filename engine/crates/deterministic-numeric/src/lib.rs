//! E2.1正式确定性数值协议。

use std::cmp::Ordering;
use thiserror::Error;

pub const U53_DENOMINATOR: u64 = 1_u64 << 53;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NumericError {
    #[error("numeric input is non-finite")]
    NonFinite,
    #[error("numeric operation overflowed its target representation")]
    Overflow,
    #[error("numeric denominator must be positive")]
    InvalidDenominator,
    #[error("canonical reduction keys must be unique")]
    DuplicateKey,
    #[error("quantile range must be non-empty")]
    EmptyRange,
}

/// Divides signed integers with round-half-to-even semantics.
///
/// # Errors
/// Rejects a non-positive denominator and the unrepresentable absolute value of
/// `i128::MIN`.
pub fn div_round_half_even(numerator: i128, denominator: i128) -> Result<i128, NumericError> {
    if denominator <= 0 {
        return Err(NumericError::InvalidDenominator);
    }
    let negative = numerator.is_negative();
    let absolute = numerator.checked_abs().ok_or(NumericError::Overflow)?;
    let quotient = absolute / denominator;
    let remainder = absolute % denominator;
    let doubled = remainder.checked_mul(2).ok_or(NumericError::Overflow)?;
    let magnitude = if doubled > denominator || (doubled == denominator && quotient % 2 == 1) {
        quotient.checked_add(1).ok_or(NumericError::Overflow)?
    } else {
        quotient
    };
    Ok(if negative { -magnitude } else { magnitude })
}

/// Quantizes a finite float by a finite positive scale using half-even rounding.
///
/// # Errors
/// Rejects invalid inputs and values outside the exact integer range used by
/// the protocol.
#[allow(clippy::float_cmp)]
pub fn quantize_i64(value: f64, scale: f64) -> Result<i64, NumericError> {
    if !value.is_finite() || !scale.is_finite() {
        return Err(NumericError::NonFinite);
    }
    if scale <= 0.0 {
        return Err(NumericError::InvalidDenominator);
    }
    let scaled = value * scale;
    if !scaled.is_finite() {
        return Err(NumericError::Overflow);
    }
    let absolute = scaled.abs();
    if absolute > 9_007_199_254_740_991.0 {
        return Err(NumericError::Overflow);
    }
    let floor = absolute.floor();
    let fraction = absolute - floor;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let base = floor as u64;
    let magnitude = if fraction > 0.5 || (fraction == 0.5 && base % 2 == 1) {
        base + 1
    } else {
        base
    };
    let signed = i64::try_from(magnitude).map_err(|_| NumericError::Overflow)?;
    Ok(if scaled.is_sign_negative() {
        -signed
    } else {
        signed
    })
}

/// Reduces finite values after strict canonical-key sorting.
///
/// # Errors
/// Rejects duplicate keys and non-finite payloads.
pub fn compensated_sum_by_key(values: &[(u64, f64)]) -> Result<f64, NumericError> {
    let mut ordered = values.to_vec();
    ordered.sort_by_key(|(key, _)| *key);
    if ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(NumericError::DuplicateKey);
    }
    let mut sum = 0.0;
    let mut correction = 0.0;
    for (_, value) in ordered {
        if !value.is_finite() {
            return Err(NumericError::NonFinite);
        }
        let adjusted = value - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(NumericError::Overflow)
    }
}

/// Compares non-negative ratios without floating-point division.
///
/// # Errors
/// Rejects zero denominators or cross-product overflow.
pub fn compare_ratios(
    left_numerator: u128,
    left_denominator: u128,
    right_numerator: u128,
    right_denominator: u128,
) -> Result<Ordering, NumericError> {
    if left_denominator == 0 || right_denominator == 0 {
        return Err(NumericError::InvalidDenominator);
    }
    let left = left_numerator
        .checked_mul(right_denominator)
        .ok_or(NumericError::Overflow)?;
    let right = right_numerator
        .checked_mul(left_denominator)
        .ok_or(NumericError::Overflow)?;
    Ok(left.cmp(&right))
}

#[must_use]
pub const fn u64_to_u53(sample: u64) -> u64 {
    sample >> 11
}

/// Maps a U53 sample into `[minimum, maximum_exclusive)` without floating point.
///
/// # Errors
/// Rejects an empty range or a width whose offset cannot fit `u64`.
pub fn map_u53_to_range(
    sample_u53: u64,
    minimum: u64,
    maximum_exclusive: u64,
) -> Result<u64, NumericError> {
    let width = maximum_exclusive
        .checked_sub(minimum)
        .filter(|width| *width > 0)
        .ok_or(NumericError::EmptyRange)?;
    let bounded = sample_u53.min(U53_DENOMINATOR - 1);
    let offset = (u128::from(bounded) * u128::from(width)) / u128::from(U53_DENOMINATOR);
    minimum
        .checked_add(u64::try_from(offset).map_err(|_| NumericError::Overflow)?)
        .ok_or(NumericError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_even_is_symmetric_and_rejects_bad_denominators() {
        assert_eq!(div_round_half_even(5, 2), Ok(2));
        assert_eq!(div_round_half_even(7, 2), Ok(4));
        assert_eq!(div_round_half_even(-5, 2), Ok(-2));
        assert_eq!(div_round_half_even(-7, 2), Ok(-4));
        assert_eq!(
            div_round_half_even(1, 0),
            Err(NumericError::InvalidDenominator)
        );
    }

    #[test]
    fn quantization_uses_half_even_and_rejects_non_finite_values() {
        assert_eq!(quantize_i64(2.5, 1.0), Ok(2));
        assert_eq!(quantize_i64(3.5, 1.0), Ok(4));
        assert_eq!(quantize_i64(-2.5, 1.0), Ok(-2));
        assert_eq!(quantize_i64(f64::NAN, 1.0), Err(NumericError::NonFinite));
        assert_eq!(
            quantize_i64(1.0, f64::INFINITY),
            Err(NumericError::NonFinite)
        );
    }

    #[test]
    fn keyed_sum_is_order_independent_and_rejects_duplicates() {
        let values = vec![(3, -1.0e16), (1, 1.0e16), (4, 2.0), (2, 1.0)];
        let mut reversed = values.clone();
        reversed.reverse();
        assert_eq!(
            compensated_sum_by_key(&values).unwrap().to_bits(),
            compensated_sum_by_key(&reversed).unwrap().to_bits()
        );
        assert_eq!(
            compensated_sum_by_key(&[(1, 2.0), (1, 3.0)]),
            Err(NumericError::DuplicateKey)
        );
    }

    #[test]
    fn ratio_and_quantile_mapping_are_integer_only() {
        assert_eq!(compare_ratios(1, 3, 2, 5), Ok(Ordering::Less));
        assert_eq!(map_u53_to_range(0, 20, 30), Ok(20));
        assert_eq!(map_u53_to_range(U53_DENOMINATOR - 1, 20, 30), Ok(29));
    }
}
