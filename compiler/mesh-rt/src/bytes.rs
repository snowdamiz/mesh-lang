//! Binary-safe, GC-managed byte values for the Mesh runtime.
//!
//! `MeshBytes` deliberately has no implicit relationship with `MeshString`.
//! UTF-8 conversion is explicit and fallible in the bytes-to-string direction.

use std::ptr;

use base64::{engine::general_purpose, Engine as _};
use subtle::ConstantTimeEq;

use crate::collections::list::{
    mesh_list_builder_new, mesh_list_builder_push, mesh_list_get, mesh_list_length,
};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};
use crate::wide_num::{mesh_u64_new, mesh_u64_value, MeshWideNum};

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

#[repr(C)]
pub struct MeshBytes {
    pub len: u64,
    // bytes follow immediately after this header
}

impl MeshBytes {
    const HEADER_SIZE: usize = std::mem::size_of::<u64>();

    unsafe fn data_ptr(&self) -> *const u8 {
        (self as *const Self as *const u8).add(Self::HEADER_SIZE)
    }

    unsafe fn data_ptr_mut(&mut self) -> *mut u8 {
        (self as *mut Self as *mut u8).add(Self::HEADER_SIZE)
    }

    pub(crate) unsafe fn as_slice(&self) -> &[u8] {
        std::slice::from_raw_parts(self.data_ptr(), self.len as usize)
    }
}

fn error(message: &str) -> *mut MeshResult {
    alloc_result(
        1,
        mesh_string_new(message.as_ptr(), message.len() as u64) as *mut u8,
    )
}

fn ok_bytes(bytes: &[u8]) -> *mut MeshResult {
    alloc_result(
        0,
        mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64) as *mut u8,
    )
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

fn ok_u64(value: u64) -> *mut MeshResult {
    alloc_result(0, mesh_u64_new(value).cast())
}

fn allocate(len: usize) -> *mut MeshBytes {
    let Some(total) = MeshBytes::HEADER_SIZE.checked_add(len) else {
        return ptr::null_mut();
    };
    if total > isize::MAX as usize {
        return ptr::null_mut();
    }

    unsafe {
        let value = mesh_gc_alloc_actor(total as u64, 8) as *mut MeshBytes;
        (*value).len = len as u64;
        value
    }
}

fn base58_encode(input: &[u8]) -> String {
    let zeroes = input.iter().take_while(|byte| **byte == 0).count();
    let mut digits = vec![0u8];
    for byte in input {
        let mut carry = *byte as u32;
        for digit in &mut digits {
            let value = (*digit as u32) * 256 + carry;
            *digit = (value % 58) as u8;
            carry = value / 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }

    let mut encoded = String::with_capacity(zeroes + digits.len());
    encoded.extend(std::iter::repeat_n('1', zeroes));
    encoded.extend(
        digits
            .iter()
            .rev()
            .skip_while(|digit| **digit == 0)
            .map(|digit| BASE58[*digit as usize] as char),
    );
    encoded
}

fn base58_decode(input: &str) -> Result<Vec<u8>, ()> {
    let zeroes = input.bytes().take_while(|byte| *byte == b'1').count();
    let mut bytes = vec![0u8];
    for encoded in input.bytes() {
        let mut carry = BASE58
            .iter()
            .position(|candidate| *candidate == encoded)
            .ok_or(())? as u32;
        for byte in &mut bytes {
            let value = (*byte as u32) * 58 + carry;
            *byte = value as u8;
            carry = value >> 8;
        }
        while carry > 0 {
            bytes.push(carry as u8);
            carry >>= 8;
        }
    }

    let mut decoded = Vec::with_capacity(zeroes + bytes.len());
    decoded.resize(zeroes, 0);
    decoded.extend(bytes.iter().rev().skip_while(|byte| **byte == 0));
    Ok(decoded)
}

/// Native ABI constructor. `data` is borrowed only for this call.
///
/// A null `data` pointer is valid only when `len` is zero.
#[no_mangle]
pub extern "C" fn mesh_bytes_new(data: *const u8, len: u64) -> *mut MeshBytes {
    let Ok(len) = usize::try_from(len) else {
        return ptr::null_mut();
    };
    if data.is_null() && len != 0 {
        return ptr::null_mut();
    }
    let value = allocate(len);
    if value.is_null() {
        return ptr::null_mut();
    }

    unsafe {
        if len != 0 {
            ptr::copy_nonoverlapping(data, (*value).data_ptr_mut(), len);
        }
        value
    }
}

/// Native package ABI: copy a borrowed native buffer into a managed `Bytes`.
#[no_mangle]
pub extern "C" fn mesh_bytes_copy_from(data: *const u8, len: u64) -> *mut MeshBytes {
    mesh_bytes_new(data, len)
}

/// Native package ABI: copy a checked range into caller-owned memory.
///
/// Returns the copied byte count, or `-1` for a null pointer/out-of-bounds range.
#[no_mangle]
pub extern "C" fn mesh_bytes_copy_to(
    bytes: *const MeshBytes,
    offset: i64,
    destination: *mut u8,
    len: u64,
) -> i64 {
    if bytes.is_null() || destination.is_null() || offset < 0 || len > i64::MAX as u64 {
        return -1;
    }
    let offset = offset as u64;
    unsafe {
        if offset.checked_add(len).is_none_or(|end| end > (*bytes).len) {
            return -1;
        }
        ptr::copy_nonoverlapping(
            (*bytes).data_ptr().add(offset as usize),
            destination,
            len as usize,
        );
    }
    len as i64
}

#[no_mangle]
pub extern "C" fn mesh_bytes_empty() -> *mut MeshBytes {
    mesh_bytes_new(ptr::null(), 0)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_from_list(values: *mut u8) -> *mut MeshResult {
    if values.is_null() {
        return error("invalid byte list");
    }
    let len = mesh_list_length(values);
    let Ok(len) = usize::try_from(len) else {
        return error("byte length overflow");
    };
    for index in 0..len {
        if mesh_list_get(values, index as i64) > u8::MAX as u64 {
            return error("byte value out of range");
        }
    }
    let bytes = allocate(len);
    if bytes.is_null() {
        return error("byte length overflow");
    }
    unsafe {
        for index in 0..len {
            *(*bytes).data_ptr_mut().add(index) = mesh_list_get(values, index as i64) as u8;
        }
    }
    alloc_result(0, bytes.cast())
}

#[no_mangle]
pub extern "C" fn mesh_bytes_to_list(bytes: *const MeshBytes) -> *mut u8 {
    if bytes.is_null() {
        return mesh_list_builder_new(0);
    }
    unsafe {
        let values = mesh_list_builder_new((*bytes).len as i64);
        for byte in (*bytes).as_slice() {
            mesh_list_builder_push(values, *byte as u64);
        }
        values
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_repeat(byte: i64, count: i64) -> *mut MeshResult {
    if !(0..=u8::MAX as i64).contains(&byte) {
        return error("byte value out of range");
    }
    let Ok(count) = usize::try_from(count) else {
        return error("byte count out of range");
    };
    let bytes = allocate(count);
    if bytes.is_null() {
        return error("byte length overflow");
    }
    unsafe {
        ptr::write_bytes((*bytes).data_ptr_mut(), byte as u8, count);
    }
    alloc_result(0, bytes.cast())
}

#[no_mangle]
pub extern "C" fn mesh_bytes_length(bytes: *const MeshBytes) -> i64 {
    unsafe { (*bytes).len as i64 }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_get(bytes: *const MeshBytes, index: i64) -> *mut MeshResult {
    unsafe {
        if index < 0 || index as u64 >= (*bytes).len {
            return error("byte index out of bounds");
        }
        ok_int(*(*bytes).data_ptr().add(index as usize) as i64)
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_slice(
    bytes: *const MeshBytes,
    start: i64,
    len: i64,
) -> *mut MeshResult {
    if start < 0 || len < 0 {
        return error("byte slice out of bounds");
    }
    unsafe {
        let start = start as u64;
        let len = len as u64;
        if start.checked_add(len).is_none_or(|end| end > (*bytes).len) {
            return error("byte slice out of bounds");
        }
        ok_bytes(std::slice::from_raw_parts(
            (*bytes).data_ptr().add(start as usize),
            len as usize,
        ))
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_concat(
    left: *const MeshBytes,
    right: *const MeshBytes,
) -> *mut MeshResult {
    unsafe {
        let Some(len) = (*left).len.checked_add((*right).len) else {
            return error("byte length overflow");
        };
        if len > (isize::MAX as usize - MeshBytes::HEADER_SIZE) as u64 {
            return error("byte length overflow");
        }
        let value = allocate(len as usize);
        ptr::copy_nonoverlapping(
            (*left).data_ptr(),
            (*value).data_ptr_mut(),
            (*left).len as usize,
        );
        ptr::copy_nonoverlapping(
            (*right).data_ptr(),
            (*value).data_ptr_mut().add((*left).len as usize),
            (*right).len as usize,
        );
        alloc_result(0, value as *mut u8)
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_secure_equals(left: *const MeshBytes, right: *const MeshBytes) -> i8 {
    unsafe {
        let left = (*left).as_slice();
        let right = (*right).as_slice();
        left.ct_eq(right).unwrap_u8() as i8
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_from_utf8(text: *const MeshString) -> *mut MeshBytes {
    unsafe { mesh_bytes_new((*text).as_bytes().as_ptr(), (*text).len) }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_to_utf8(bytes: *const MeshBytes) -> *mut MeshResult {
    unsafe {
        match std::str::from_utf8((*bytes).as_slice()) {
            Ok(text) => alloc_result(
                0,
                mesh_string_new(text.as_ptr(), text.len() as u64) as *mut u8,
            ),
            Err(_) => error("invalid utf-8"),
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_to_base64(bytes: *const MeshBytes) -> *mut MeshString {
    unsafe {
        let encoded = general_purpose::STANDARD.encode((*bytes).as_slice());
        mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_from_base64(text: *const MeshString) -> *mut MeshResult {
    unsafe {
        let decoded = general_purpose::STANDARD
            .decode((*text).as_str())
            .or_else(|_| general_purpose::STANDARD_NO_PAD.decode((*text).as_str()));
        decoded
            .map(|bytes| ok_bytes(&bytes))
            .unwrap_or_else(|_| error("invalid base64"))
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_to_base58(bytes: *const MeshBytes) -> *mut MeshString {
    unsafe {
        let encoded = base58_encode((*bytes).as_slice());
        mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_from_base58(text: *const MeshString) -> *mut MeshResult {
    unsafe {
        base58_decode((*text).as_str())
            .map(|bytes| ok_bytes(&bytes))
            .unwrap_or_else(|_| error("invalid base58"))
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_to_hex(bytes: *const MeshBytes) -> *mut MeshString {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    unsafe {
        let bytes = (*bytes).as_slice();
        let mut encoded = Vec::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push(HEX[(byte >> 4) as usize]);
            encoded.push(HEX[(byte & 0x0f) as usize]);
        }
        mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_from_hex(text: *const MeshString) -> *mut MeshResult {
    unsafe {
        let text = (*text).as_str();
        if text.len() % 2 != 0 {
            return error("invalid hex");
        }
        let mut decoded = Vec::with_capacity(text.len() / 2);
        for pair in text.as_bytes().chunks_exact(2) {
            let Some(high) = (pair[0] as char).to_digit(16) else {
                return error("invalid hex");
            };
            let Some(low) = (pair[1] as char).to_digit(16) else {
                return error("invalid hex");
            };
            decoded.push(((high << 4) | low) as u8);
        }
        ok_bytes(&decoded)
    }
}

fn read_uint(
    bytes: *const MeshBytes,
    offset: i64,
    width: usize,
    big_endian: bool,
) -> Result<u64, &'static str> {
    if bytes.is_null() || offset < 0 {
        return Err("unsigned integer read out of bounds");
    }
    unsafe {
        let offset = offset as u64;
        if offset
            .checked_add(width as u64)
            .is_none_or(|end| end > (*bytes).len)
        {
            return Err("unsigned integer read out of bounds");
        }
        let mut encoded = [0u8; 8];
        let destination = if big_endian {
            encoded[8 - width..].as_mut_ptr()
        } else {
            encoded.as_mut_ptr()
        };
        ptr::copy_nonoverlapping((*bytes).data_ptr().add(offset as usize), destination, width);
        Ok(if big_endian {
            u64::from_be_bytes(encoded)
        } else {
            u64::from_le_bytes(encoded)
        })
    }
}

fn read_int_result(
    bytes: *const MeshBytes,
    offset: i64,
    width: usize,
    big_endian: bool,
) -> *mut MeshResult {
    read_uint(bytes, offset, width, big_endian)
        .map(|value| ok_int(value as i64))
        .unwrap_or_else(error)
}

fn read_u64_result(
    bytes: *const MeshBytes,
    offset: i64,
    width: usize,
    big_endian: bool,
) -> *mut MeshResult {
    read_uint(bytes, offset, width, big_endian)
        .map(ok_u64)
        .unwrap_or_else(error)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u16_be(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_int_result(bytes, offset, 2, true)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u16_le(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_int_result(bytes, offset, 2, false)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u32_be(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_u64_result(bytes, offset, 4, true)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u32_le(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_u64_result(bytes, offset, 4, false)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u64_be(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_u64_result(bytes, offset, 8, true)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_u64_le(bytes: *const MeshBytes, offset: i64) -> *mut MeshResult {
    read_u64_result(bytes, offset, 8, false)
}

fn write_uint(value: u64, width: usize, big_endian: bool) -> *mut MeshResult {
    if width < 8 && value >= (1u64 << (width * 8)) {
        return error("unsigned integer does not fit width");
    }
    let encoded = if big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    if big_endian {
        ok_bytes(&encoded[8 - width..])
    } else {
        ok_bytes(&encoded[..width])
    }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_write_u16_be(value: i64) -> *mut MeshResult {
    if value < 0 {
        return error("unsigned integer does not fit width");
    }
    write_uint(value as u64, 2, true)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_write_u32_be(value: *const MeshWideNum) -> *mut MeshResult {
    if value.is_null() {
        return error("invalid unsigned integer");
    }
    unsafe { write_uint(mesh_u64_value(value), 4, true) }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_write_u64_be(value: *const MeshWideNum) -> *mut MeshResult {
    if value.is_null() {
        return error("invalid unsigned integer");
    }
    unsafe { write_uint(mesh_u64_value(value), 8, true) }
}

#[no_mangle]
pub extern "C" fn mesh_bytes_read_uint_le(
    bytes: *const MeshBytes,
    offset: i64,
    width: i64,
) -> *mut MeshResult {
    if offset < 0 || !matches!(width, 1 | 2 | 4 | 8) {
        return error("invalid unsigned integer width or offset");
    }
    read_uint(bytes, offset, width as usize, false)
        .map(|value| {
            let value = value.to_string();
            alloc_result(
                0,
                mesh_string_new(value.as_ptr(), value.len() as u64) as *mut u8,
            )
        })
        .unwrap_or_else(error)
}

#[no_mangle]
pub extern "C" fn mesh_bytes_write_uint_le(
    value: *const MeshString,
    width: i64,
) -> *mut MeshResult {
    if !matches!(width, 1 | 2 | 4 | 8) {
        return error("invalid unsigned integer width");
    }
    unsafe {
        let Ok(value) = (*value).as_str().parse::<u64>() else {
            return error("invalid unsigned integer");
        };
        write_uint(value, width as usize, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;
    use rand::{rngs::StdRng, Rng, SeedableRng};

    #[test]
    fn binary_codecs_and_native_copy_round_trip() {
        mesh_rt_init();
        assert!(mesh_bytes_copy_from(ptr::null(), 1).is_null());
        let input = [0, 0xff, 0x41, 0];
        let bytes = mesh_bytes_copy_from(input.as_ptr(), input.len() as u64);
        let mut output = [0; 2];
        assert_eq!(mesh_bytes_copy_to(bytes, 1, output.as_mut_ptr(), 2), 2);
        assert_eq!(output, [0xff, 0x41]);
        assert_eq!(mesh_bytes_copy_to(bytes, 3, output.as_mut_ptr(), 2), -1);
        assert_eq!(base58_encode(b"Hello World"), "JxF12TrwUP45BMd");
        assert_eq!(base58_encode(&[0, 0, 1]), "112");
        assert_eq!(base58_decode("112"), Ok(vec![0, 0, 1]));
        assert_eq!(base58_decode("0"), Err(()));
        let short = mesh_bytes_copy_from([0].as_ptr(), 1);
        let long = mesh_bytes_copy_from([0; 257].as_ptr(), 257);
        assert_eq!(mesh_bytes_secure_equals(short, long), 0);

        unsafe {
            let encoded = mesh_bytes_to_base58(bytes);
            let decoded = mesh_bytes_from_base58(encoded);
            assert_eq!((*decoded).tag, 0);
            assert_eq!(
                (*((*decoded).value as *const MeshBytes)).as_slice(),
                input.as_slice()
            );
        }
    }

    #[test]
    fn hostile_byte_ranges_never_panic_or_bypass_bounds() {
        mesh_rt_init();
        let mut rng = StdRng::seed_from_u64(0x4d45_5348_4259_5445);

        for _ in 0..512 {
            let len = rng.random_range(0..=64usize);
            let input: Vec<u8> = (0..len).map(|_| rng.random()).collect();
            let bytes = mesh_bytes_copy_from(input.as_ptr(), len as u64);
            let offset = match rng.random_range(0..4) {
                0 => -1,
                1 => len as i64,
                2 => len as i64 + 1,
                _ => rng.random_range(0..=len as i64),
            };

            for (width, read) in [
                (2, mesh_bytes_read_u16_be as extern "C" fn(_, _) -> _),
                (2, mesh_bytes_read_u16_le),
                (4, mesh_bytes_read_u32_be),
                (4, mesh_bytes_read_u32_le),
                (8, mesh_bytes_read_u64_be),
                (8, mesh_bytes_read_u64_le),
            ] {
                let result = read(bytes, offset);
                let in_bounds = offset >= 0
                    && (offset as usize)
                        .checked_add(width)
                        .is_some_and(|end| end <= len);
                unsafe { assert_eq!((*result).tag == 0, in_bounds) };
            }

            let slice_len = rng.random_range(-1..=len as i64 + 1);
            let slice = mesh_bytes_slice(bytes, offset, slice_len);
            let in_bounds = offset >= 0
                && slice_len >= 0
                && (offset as usize)
                    .checked_add(slice_len as usize)
                    .is_some_and(|end| end <= len);
            unsafe { assert_eq!((*slice).tag == 0, in_bounds) };
        }
    }
}
