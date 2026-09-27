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
    use super::*;
    use crate::gc::mesh_rt_init;
    use crate::string::mesh_str;

    /// Every mode on a remainder of exactly one half, both signs, and on a
    /// remainder below one half.
    #[test]
    fn each_rounding_mode_rounds_both_signs() {
        let modes = [
            "toward_zero",
            "floor",
            "ceil",
            "half_away_from_zero",
            "half_even",
        ];
        let rounded = |numerator| modes.map(|mode| checked_mul_div(numerator, 1, 2, mode));
        assert_eq!(rounded(7), [Ok(3), Ok(3), Ok(4), Ok(4), Ok(4)]);
        assert_eq!(rounded(-7), [Ok(-3), Ok(-4), Ok(-3), Ok(-4), Ok(-4)]);
        assert_eq!(rounded(5)[3..], [Ok(3), Ok(2)]);
        assert_eq!(checked_mul_div(4, 1, 3, "half_away_from_zero"), Ok(1));
    }

    #[test]
    fn division_and_scaling_refuse_what_they_cannot_represent() {
        assert_eq!(checked_mul_div(1, 1, 0, "floor"), Err("division by zero"));
        assert_eq!(
            checked_mul_div(i64::MAX, 2, 1, "floor"),
            Err("integer overflow")
        );
        assert_eq!(
            checked_rescale(1, -1, 2, "floor"),
            Err("scale must be nonnegative")
        );
        assert_eq!(
            checked_rescale(1, 0, i64::MAX, "floor"),
            Err("scale out of range")
        );
        assert_eq!(
            checked_rescale(1, 0, 39, "floor"),
            Err("scale out of range")
        );
        assert_eq!(
            checked_rescale(i64::MAX, 0, 38, "floor"),
            Err("integer overflow")
        );
        assert_eq!(
            checked_rescale(i64::MAX, 0, 1, "floor"),
            Err("integer overflow")
        );
    }

    /// The entry points return each result as a Mesh `Result`.
    #[test]
    fn entry_points_return_mesh_results() {
        mesh_rt_init();
        let int = |result: *mut MeshResult| unsafe {
            assert_eq!((*result).tag, 0);
            *(*result).value.cast::<i64>()
        };
        let error = |result: *mut MeshResult| unsafe {
            assert_eq!((*result).tag, 1);
            (*(*result).value.cast::<MeshString>()).as_str().to_string()
        };
        assert_eq!(int(mesh_checked_add(2, 3)), 5);
        assert_eq!(error(mesh_checked_add(i64::MAX, 1)), "integer overflow");
        assert_eq!(int(mesh_checked_sub(2, 3)), -1);
        assert_eq!(int(mesh_checked_mul(2, 3)), 6);
        assert_eq!(int(mesh_checked_div(7, 2)), 3);
        assert_eq!(error(mesh_checked_div(7, 0)), "division by zero");
        assert_eq!(error(mesh_checked_div(i64::MIN, -1)), "integer overflow");
        assert_eq!(int(mesh_checked_abs(-4)), 4);
        let half_even = mesh_str("half_even");
        assert_eq!(int(mesh_checked_mul_div(5, 3, 2, half_even)), 8);
        assert_eq!(int(mesh_checked_rescale(12355, 3, 2, half_even)), 1236);
    }

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
