//! Process-local monotonic time and checked duration helpers.

use std::sync::OnceLock;
use std::time::Instant;

use crate::io::{alloc_result, MeshResult};
use crate::string::mesh_string_new;

static ORIGIN: OnceLock<Instant> = OnceLock::new();

fn result(value: Result<i64, &'static str>) -> *mut MeshResult {
    match value {
        Ok(value) => alloc_result(0, Box::into_raw(Box::new(value)).cast()),
        Err(error) => alloc_result(
            1,
            mesh_string_new(error.as_ptr(), error.len() as u64).cast(),
        ),
    }
}

#[no_mangle]
pub extern "C" fn mesh_monotonic_now_nanos() -> i64 {
    i64::try_from(ORIGIN.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(i64::MAX)
}

#[no_mangle]
pub extern "C" fn mesh_monotonic_elapsed(start: i64, finish: i64) -> *mut MeshResult {
    result(
        finish
            .checked_sub(start)
            .filter(|elapsed| *elapsed >= 0)
            .ok_or("monotonic finish precedes start"),
    )
}

#[no_mangle]
pub extern "C" fn mesh_duration_millis(value: i64) -> *mut MeshResult {
    result(
        value
            .checked_mul(1_000_000)
            .filter(|duration| *duration >= 0)
            .ok_or("invalid duration"),
    )
}

#[no_mangle]
pub extern "C" fn mesh_duration_seconds(value: i64) -> *mut MeshResult {
    result(
        value
            .checked_mul(1_000_000_000)
            .filter(|duration| *duration >= 0)
            .ok_or("invalid duration"),
    )
}

#[cfg(test)]
mod tests {
    use super::mesh_monotonic_now_nanos;

    #[test]
    fn clock_never_moves_backwards() {
        let first = mesh_monotonic_now_nanos();
        let second = mesh_monotonic_now_nanos();
        assert!(second >= first);
    }
}
