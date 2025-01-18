use num::{BigUint, Integer, Zero};

/// Converts an unsigned, fixed-point value to a string.
/// The output is equivalent to `real(uint / (2 ** lg_scaling_factor)).to_string()`.
pub(crate) fn big_uint_to_ufixed(uint: &BigUint, lg_scaling_factor: i64) -> String {
    if lg_scaling_factor == 0 {
        return format!("{uint}");
    } else if lg_scaling_factor < 0 {
        return format!("{}", uint * 2_u32.pow(-lg_scaling_factor as u32));
    }
    // Compute the scaling divisor (2 ** lg_scaling_factor)
    let divisor = BigUint::from(1_u32) << lg_scaling_factor;

    // Perform the integer division and remainder
    let (integer_part, mut remainder) = uint.div_rem(&divisor);

    if remainder.is_zero() {
        integer_part.to_string() // No fractional part
    } else {
        let mut fractional_part = String::new();

        // Scale up the remainder to extract fractional digits
        for _ in 0..lg_scaling_factor {
            remainder *= 10_u32;
            let digit = &remainder >> lg_scaling_factor;
            fractional_part.push_str(&digit.to_string());
            remainder %= &divisor;

            // Stop if the scaled remainder becomes zero
            if remainder.is_zero() {
                break;
            }
        }

        format!("{}.{}", integer_part, fractional_part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn check(value: impl Into<BigUint>, lg_scaling_factor: i64, expected: impl Into<String>) {
        let value = value.into();
        let result = big_uint_to_ufixed(&value, lg_scaling_factor);
        assert_eq!(result, expected.into());
    }

    #[test]
    fn test_exact_integer() {
        check(256_u32, 8, "1")
    }

    #[test]
    fn test_fractional_value() {
        check(48225_u32, 8, "188.37890625");
        check(100_u32, 10, "0.09765625");
        check(8192_u32, 15, "0.25");
        check(16384_u32, 15, "0.5");
    }

    #[test]
    fn test_large_value() {
        check(
            BigUint::from_str("12345678901234567890").unwrap(),
            20,
            "11773756886705.9401416778564453125",
        )
    }

    #[test]
    fn test_value_less_than_one() {
        check(1_u32, 10, "0.0009765625")
    }

    #[test]
    fn test_zero_value() {
        check(0_u32, 16, "0")
    }

    #[test]
    fn test_negative_scaling_factor() {
        check(500_u32, -1, "1000")
    }
}
