//! Checked opaque integer values wider than Mesh's signed `Int`.

use std::cmp::Ordering;

use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, err_result, ok_int, MeshResult};
use crate::string::{mesh_str, MeshString};

#[repr(C)]
pub struct MeshWideNum {
    low: u64,
    high: u64,
}

pub(crate) fn mesh_u64_new(value: u64) -> *mut MeshWideNum {
    allocate(value as u128)
}

pub(crate) unsafe fn mesh_u64_value(value: *const MeshWideNum) -> u64 {
    bits(value) as u64
}

fn allocate(bits: u128) -> *mut MeshWideNum {
    unsafe {
        let value = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshWideNum>() as u64,
            std::mem::align_of::<MeshWideNum>() as u64,
        ) as *mut MeshWideNum;
        (*value).low = bits as u64;
        (*value).high = (bits >> 64) as u64;
        value
    }
}

unsafe fn bits(value: *const MeshWideNum) -> u128 {
    ((*value).high as u128) << 64 | (*value).low as u128
}

fn ok_wide(value: u128) -> *mut MeshResult {
    alloc_result(0, allocate(value) as *mut u8)
}

fn ordering(value: Ordering) -> i64 {
    match value {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

fn mesh_string(value: impl ToString) -> *mut MeshString {
    let value = value.to_string();
    mesh_str(&value)
}

macro_rules! wide_abi {
    (
        $rust_type:ty,
        $label:literal,
        $parse:ident,
        $compare:ident,
        $add:ident,
        $subtract:ident,
        $multiply:ident,
        $divide:ident,
        $to_int:ident,
        $to_string:ident
    ) => {
        #[no_mangle]
        pub extern "C" fn $parse(text: *const MeshString) -> *mut MeshResult {
            unsafe {
                match (*text).as_str().parse::<$rust_type>() {
                    Ok(value) => ok_wide(value as u128),
                    Err(_) => err_result(concat!("invalid ", $label)),
                }
            }
        }

        #[no_mangle]
        pub extern "C" fn $compare(left: *const MeshWideNum, right: *const MeshWideNum) -> i64 {
            unsafe { ordering((bits(left) as $rust_type).cmp(&(bits(right) as $rust_type))) }
        }

        #[no_mangle]
        pub extern "C" fn $add(
            left: *const MeshWideNum,
            right: *const MeshWideNum,
        ) -> *mut MeshResult {
            unsafe {
                (bits(left) as $rust_type)
                    .checked_add(bits(right) as $rust_type)
                    .map(|value| ok_wide(value as u128))
                    .unwrap_or_else(|| err_result(concat!($label, " addition overflow")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $subtract(
            left: *const MeshWideNum,
            right: *const MeshWideNum,
        ) -> *mut MeshResult {
            unsafe {
                (bits(left) as $rust_type)
                    .checked_sub(bits(right) as $rust_type)
                    .map(|value| ok_wide(value as u128))
                    .unwrap_or_else(|| err_result(concat!($label, " subtraction overflow")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $multiply(
            left: *const MeshWideNum,
            right: *const MeshWideNum,
        ) -> *mut MeshResult {
            unsafe {
                (bits(left) as $rust_type)
                    .checked_mul(bits(right) as $rust_type)
                    .map(|value| ok_wide(value as u128))
                    .unwrap_or_else(|| err_result(concat!($label, " multiplication overflow")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $divide(
            left: *const MeshWideNum,
            right: *const MeshWideNum,
        ) -> *mut MeshResult {
            unsafe {
                let right = bits(right) as $rust_type;
                if right == 0 {
                    return err_result(concat!($label, " division by zero"));
                }
                (bits(left) as $rust_type)
                    .checked_div(right)
                    .map(|value| ok_wide(value as u128))
                    .unwrap_or_else(|| err_result(concat!($label, " division overflow")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $to_int(value: *const MeshWideNum) -> *mut MeshResult {
            unsafe {
                i64::try_from(bits(value) as $rust_type)
                    .map(ok_int)
                    .unwrap_or_else(|_| err_result(concat!($label, " does not fit Int")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $to_string(value: *const MeshWideNum) -> *mut MeshString {
            unsafe { mesh_string(bits(value) as $rust_type) }
        }
    };
}

wide_abi!(
    u64,
    "u64",
    mesh_u64_parse,
    mesh_u64_compare,
    mesh_u64_add,
    mesh_u64_subtract,
    mesh_u64_multiply,
    mesh_u64_divide,
    mesh_u64_to_int,
    mesh_u64_to_string
);

wide_abi!(
    u128,
    "u128",
    mesh_u128_parse,
    mesh_u128_compare,
    mesh_u128_add,
    mesh_u128_subtract,
    mesh_u128_multiply,
    mesh_u128_divide,
    mesh_u128_to_int,
    mesh_u128_to_string
);

wide_abi!(
    i128,
    "i128",
    mesh_i128_parse,
    mesh_i128_compare,
    mesh_i128_add,
    mesh_i128_subtract,
    mesh_i128_multiply,
    mesh_i128_divide,
    mesh_i128_to_int,
    mesh_i128_to_string
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    unsafe fn value(result: *mut MeshResult) -> *const MeshWideNum {
        assert_eq!((*result).tag, 0, "expected Ok");
        (*result).value.cast()
    }

    unsafe fn error(result: *mut MeshResult) -> String {
        assert_eq!((*result).tag, 1, "expected Err");
        (*(*result).value.cast::<MeshString>()).as_str().to_string()
    }

    unsafe fn text(value: *mut MeshString) -> String {
        (*value).as_str().to_string()
    }

    /// Each operation of the ABI all three widths share, on the signed
    /// width: the only one whose division can overflow.
    #[test]
    fn checked_arithmetic_refuses_what_does_not_fit() {
        mesh_rt_init();
        unsafe {
            let parse = |text: &str| value(mesh_i128_parse(mesh_str(text)));
            let minimum = parse(&i128::MIN.to_string());
            let minus_one = parse("-1");
            let two = parse("2");
            let zero = parse("0");
            let large = parse(&(i128::from(i64::MAX) + 1).to_string());
            let show = |result| text(mesh_i128_to_string(value(result)));

            assert_eq!(error(mesh_i128_parse(mesh_str("2x"))), "invalid i128");
            assert_eq!(
                [
                    mesh_i128_compare(minimum, two),
                    mesh_i128_compare(two, two),
                    mesh_i128_compare(two, minus_one),
                ],
                [-1, 0, 1]
            );
            assert_eq!(show(mesh_i128_add(two, minus_one)), "1");
            assert_eq!(
                error(mesh_i128_add(minimum, minus_one)),
                "i128 addition overflow"
            );
            assert_eq!(show(mesh_i128_subtract(two, two)), "0");
            assert_eq!(
                error(mesh_i128_subtract(minimum, two)),
                "i128 subtraction overflow"
            );
            assert_eq!(show(mesh_i128_multiply(two, two)), "4");
            assert_eq!(
                error(mesh_i128_multiply(minimum, two)),
                "i128 multiplication overflow"
            );
            assert_eq!(show(mesh_i128_divide(two, minus_one)), "-2");
            assert_eq!(error(mesh_i128_divide(two, zero)), "i128 division by zero");
            assert_eq!(
                error(mesh_i128_divide(minimum, minus_one)),
                "i128 division overflow"
            );
            let int = mesh_i128_to_int(minus_one);
            assert_eq!(((*int).tag, *(*int).value.cast::<i64>()), (0, -1));
            assert_eq!(error(mesh_i128_to_int(large)), "i128 does not fit Int");
        }
    }

    /// The unsigned widths read and write their whole range, and a u64 the
    /// runtime makes reads back as itself.
    #[test]
    fn unsigned_widths_round_trip_their_extremes() {
        mesh_rt_init();
        unsafe {
            let u64_max = value(mesh_u64_parse(mesh_str(&u64::MAX.to_string())));
            assert_eq!(text(mesh_u64_to_string(u64_max)), u64::MAX.to_string());
            assert_eq!(error(mesh_u64_parse(mesh_str("-1"))), "invalid u64");
            let u128_max = value(mesh_u128_parse(mesh_str(&u128::MAX.to_string())));
            assert_eq!(text(mesh_u128_to_string(u128_max)), u128::MAX.to_string());
            assert_eq!(mesh_u64_value(mesh_u64_new(7)), 7);
        }
    }
}
