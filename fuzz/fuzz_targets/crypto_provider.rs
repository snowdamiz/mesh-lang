#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    mesh_rt::crypto::fuzz_crypto_boundaries(data);
});
