//! Checked opaque integer values wider than Mesh's signed `Int`.

use std::cmp::Ordering;

use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

#[repr(C)]
pub struct MeshWideNum {
    low: u64,
    high: u64,
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

fn error(message: &str) -> *mut MeshResult {
    alloc_result(
        1,
        mesh_string_new(message.as_ptr(), message.len() as u64) as *mut u8,
    )
}

fn ok_wide(value: u128) -> *mut MeshResult {
    alloc_result(0, allocate(value) as *mut u8)
}

fn ok_int(value: i64) -> *mut MeshResult {
    unsafe {
        let result = mesh_gc_alloc_actor(
            std::mem::size_of::<i64>() as u64,
            std::mem::align_of::<i64>() as u64,
        ) as *mut i64;
        *result = value;
        alloc_result(0, result.cast())
    }
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
    mesh_string_new(value.as_ptr(), value.len() as u64)
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
                    Err(_) => error(concat!("invalid ", $label)),
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
                    .unwrap_or_else(|| error(concat!($label, " addition overflow")))
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
                    .unwrap_or_else(|| error(concat!($label, " subtraction overflow")))
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
                    .unwrap_or_else(|| error(concat!($label, " multiplication overflow")))
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
                    return error(concat!($label, " division by zero"));
                }
                (bits(left) as $rust_type)
                    .checked_div(right)
                    .map(|value| ok_wide(value as u128))
                    .unwrap_or_else(|| error(concat!($label, " division overflow")))
            }
        }

        #[no_mangle]
        pub extern "C" fn $to_int(value: *const MeshWideNum) -> *mut MeshResult {
            unsafe {
                i64::try_from(bits(value) as $rust_type)
                    .map(ok_int)
                    .unwrap_or_else(|_| error(concat!($label, " does not fit Int")))
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
