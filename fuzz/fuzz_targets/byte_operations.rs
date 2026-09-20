#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_rt::bytes::{
    mesh_bytes_concat, mesh_bytes_copy_from, mesh_bytes_copy_to, mesh_bytes_empty,
    mesh_bytes_from_base58, mesh_bytes_from_base64, mesh_bytes_from_hex, mesh_bytes_get,
    mesh_bytes_length, mesh_bytes_read_u16_be, mesh_bytes_read_u16_le, mesh_bytes_read_u32_be,
    mesh_bytes_read_u32_le, mesh_bytes_read_u64_be, mesh_bytes_read_u64_le,
    mesh_bytes_read_uint_le, mesh_bytes_repeat, mesh_bytes_secure_equals, mesh_bytes_slice,
    mesh_bytes_to_base58, mesh_bytes_to_base64, mesh_bytes_to_hex, mesh_bytes_to_list,
    mesh_bytes_to_utf8,
};
use mesh_rt::gc::{mesh_rt_init, mesh_rt_reset_for_fuzzing};

fn hostile_offset(selector: u8, length: usize) -> i64 {
    match selector % 5 {
        0 => -1,
        1 => length as i64,
        2 => length.saturating_add(1) as i64,
        3 => i64::MAX,
        _ => (selector as usize % length.max(1)) as i64,
    }
}

fuzz_target!(|data: &[u8]| {
    let input = &data[..data.len().min(4_096)];
    mesh_rt_init();
    {
        let bytes = mesh_bytes_copy_from(input.as_ptr(), input.len() as u64);
        assert!(!bytes.is_null());
        assert_eq!(mesh_bytes_length(bytes), input.len() as i64);

        let offset = hostile_offset(input.first().copied().unwrap_or_default(), input.len());
        let length = input.get(1).copied().unwrap_or_default() as i64;
        let _ = mesh_bytes_get(bytes, offset);
        let _ = mesh_bytes_slice(bytes, offset, length);
        let _ = mesh_bytes_read_u16_be(bytes, offset);
        let _ = mesh_bytes_read_u16_le(bytes, offset);
        let _ = mesh_bytes_read_u32_be(bytes, offset);
        let _ = mesh_bytes_read_u32_le(bytes, offset);
        let _ = mesh_bytes_read_u64_be(bytes, offset);
        let _ = mesh_bytes_read_u64_le(bytes, offset);
        for width in [-1, 0, 1, 2, 4, 8, 9, i64::MAX] {
            let _ = mesh_bytes_read_uint_le(bytes, offset, width);
        }

        let mut output = [0; 64];
        let copy_length = input.get(2).copied().unwrap_or_default() as u64 % 65;
        let _ = mesh_bytes_copy_to(bytes, offset, output.as_mut_ptr(), copy_length);
        let empty = mesh_bytes_empty();
        let _ = mesh_bytes_concat(bytes, empty);
        let _ = mesh_bytes_secure_equals(bytes, empty);
        let _ = mesh_bytes_to_list(bytes);
        let _ = mesh_bytes_to_utf8(bytes);
        let repeat_count = input.get(3).copied().unwrap_or_default() as i64;
        let _ = mesh_bytes_repeat(offset, repeat_count);

        let codec_input = &input[..input.len().min(128)];
        let codec_bytes = mesh_bytes_copy_from(codec_input.as_ptr(), codec_input.len() as u64);
        let base64 = mesh_bytes_to_base64(codec_bytes);
        let base58 = mesh_bytes_to_base58(codec_bytes);
        let hex = mesh_bytes_to_hex(codec_bytes);
        let _ = mesh_bytes_from_base64(base64);
        let _ = mesh_bytes_from_base58(base58);
        let _ = mesh_bytes_from_hex(hex);
    }
    mesh_rt_reset_for_fuzzing();
});
