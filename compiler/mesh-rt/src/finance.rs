//! Checked integer arithmetic for financial calculations.

use crate::io::{err_result, ok_int, MeshResult};
use crate::string::MeshString;

fn result(value: Result<i64, &'static str>) -> *mut MeshResult {
    match value {
        Ok(value) => ok_int(value),
        Err(error) => err_result(error),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rounding {
    TowardZero,
    Floor,
    Ceil,
    HalfAwayFromZero,
    HalfEven,
}

impl Rounding {
    fn parse(mode: &str) -> Result<Self, &'static str> {
        match mode {
            "toward_zero" => Ok(Self::TowardZero),
            "floor" => Ok(Self::Floor),
            "ceil" => Ok(Self::Ceil),
            "half_away_from_zero" => Ok(Self::HalfAwayFromZero),
            "half_even" => Ok(Self::HalfEven),
            _ => Err("invalid rounding mode"),
        }
    }
}

fn round_quotient(quotient: i128, remainder: i128, denominator: i128, mode: Rounding) -> i128 {
    if remainder == 0 {
        return quotient;
    }

    let away_from_zero = if (remainder > 0) == (denominator > 0) {
        1
    } else {
        -1
    };
    match mode {
        Rounding::TowardZero => quotient,
        Rounding::Floor => {
            if away_from_zero < 0 {
                quotient - 1
            } else {
                quotient
            }
        }
        Rounding::Ceil => {
            if away_from_zero > 0 {
                quotient + 1
            } else {
                quotient
            }
        }
        Rounding::HalfAwayFromZero | Rounding::HalfEven => {
            let doubled_remainder = remainder.abs() * 2;
            let denominator = denominator.abs();
            let round_away = doubled_remainder > denominator
                || (doubled_remainder == denominator
                    && (mode == Rounding::HalfAwayFromZero || quotient % 2 != 0));
            if round_away {
                quotient + away_from_zero
            } else {
                quotient
            }
        }
    }
}

fn checked_mul_div(
    left: i64,
    right: i64,
    denominator: i64,
    mode: &str,
) -> Result<i64, &'static str> {
    let mode = Rounding::parse(mode)?;
    if denominator == 0 {
        return Err("division by zero");
    }
    let product = i128::from(left) * i128::from(right);
    let denominator = i128::from(denominator);
    let quotient = product / denominator;
    let remainder = product % denominator;
    i64::try_from(round_quotient(quotient, remainder, denominator, mode))
        .map_err(|_| "integer overflow")
}

fn checked_rescale(
    raw: i64,
    from_scale: i64,
    to_scale: i64,
    mode: &str,
) -> Result<i64, &'static str> {
    let mode = Rounding::parse(mode)?;
    if from_scale < 0 || to_scale < 0 {
        return Err("scale must be nonnegative");
    }
    let scale_difference =
        u32::try_from(from_scale.abs_diff(to_scale)).map_err(|_| "scale out of range")?;
    let factor = 10_i128
        .checked_pow(scale_difference)
        .ok_or("scale out of range")?;
    let value = if to_scale >= from_scale {
        i128::from(raw)
            .checked_mul(factor)
            .ok_or("integer overflow")?
    } else {
        let raw = i128::from(raw);
        round_quotient(raw / factor, raw % factor, factor, mode)
    };
    i64::try_from(value).map_err(|_| "integer overflow")
}

macro_rules! checked_binary {
    ($name:ident, $operation:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(left: i64, right: i64) -> *mut MeshResult {
            result(left.$operation(right).ok_or("integer overflow"))
        }
    };
}

checked_binary!(mesh_checked_add, checked_add);
checked_binary!(mesh_checked_sub, checked_sub);
checked_binary!(mesh_checked_mul, checked_mul);

#[no_mangle]
pub extern "C" fn mesh_checked_div(left: i64, right: i64) -> *mut MeshResult {
    result(if right == 0 {
        Err("division by zero")
    } else {
        left.checked_div(right).ok_or("integer overflow")
    })
}

#[no_mangle]
pub extern "C" fn mesh_checked_abs(value: i64) -> *mut MeshResult {
    result(value.checked_abs().ok_or("integer overflow"))
}

/// Checked.mul_div(a, b, denominator, rounding) -> Result<Int, String>
#[no_mangle]
pub extern "C" fn mesh_checked_mul_div(
    left: i64,
    right: i64,
    denominator: i64,
    rounding: *const MeshString,
) -> *mut MeshResult {
    let mode = unsafe { (*rounding).as_str() };
    result(checked_mul_div(left, right, denominator, mode))
}

/// Checked.rescale(raw, from_scale, to_scale, rounding) -> Result<Int, String>
#[no_mangle]
pub extern "C" fn mesh_checked_rescale(
    raw: i64,
    from_scale: i64,
    to_scale: i64,
    rounding: *const MeshString,
) -> *mut MeshResult {
    let mode = unsafe { (*rounding).as_str() };
    result(checked_rescale(raw, from_scale, to_scale, mode))
}

#[cfg(test)]
mod tests {
    use super::{checked_mul_div, checked_rescale};

    #[test]
    fn half_even_uses_a_wide_intermediate() {
        assert_eq!(checked_mul_div(5, 3, 2, "half_even"), Ok(8));
        assert_eq!(
            checked_mul_div(i64::MAX, i64::MAX, i64::MAX, "toward_zero"),
            Ok(i64::MAX)
        );
    }

    /// An unknown rounding mode is refused whether or not the result
    /// needed rounding.
    #[test]
    fn an_unknown_rounding_mode_is_refused_even_for_exact_results() {
        assert_eq!(
            checked_mul_div(4, 1, 2, "nearest"),
            Err("invalid rounding mode")
        );
        assert_eq!(
            checked_rescale(100, 2, 4, "nearest"),
            Err("invalid rounding mode")
        );
        assert_eq!(
            checked_rescale(100, 2, 1, "nearest"),
            Err("invalid rounding mode")
        );
    }

    #[test]
    fn rescale_rounds_when_reducing_precision() {
        assert_eq!(checked_rescale(12355, 3, 2, "half_even"), Ok(1236));
        assert_eq!(checked_rescale(12345, 2, 4, "toward_zero"), Ok(1_234_500));
    }
}
