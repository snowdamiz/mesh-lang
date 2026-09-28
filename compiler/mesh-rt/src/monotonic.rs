//! Process-local monotonic time and checked duration helpers.

use std::sync::OnceLock;
use std::time::Instant;

use crate::io::{err_result, ok_int, MeshResult};

static ORIGIN: OnceLock<Instant> = OnceLock::new();

fn result(value: Result<i64, &'static str>) -> *mut MeshResult {
    match value {
        Ok(value) => ok_int(value),
        Err(error) => err_result(error),
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
    use super::*;

    /// `(tag, the Int an Ok holds, or 0)`.
    fn outcome(result: *mut MeshResult) -> (u8, i64) {
        let result = unsafe { &*result };
        let value = if result.tag == 0 {
            unsafe { *(result.value as *const i64) }
        } else {
            0
        };
        (result.tag, value)
    }

    /// Durations are nanoseconds, and none is negative or past an Int.
    #[test]
    fn durations_in_nanoseconds_or_an_error() {
        crate::gc::mesh_rt_init();
        assert_eq!(outcome(mesh_duration_millis(3)), (0, 3_000_000));
        assert_eq!(outcome(mesh_duration_seconds(2)).0, 0);
        for invalid in [mesh_duration_millis(-1), mesh_duration_millis(i64::MAX)] {
            assert_eq!(outcome(invalid).0, 1);
        }
        assert_eq!(
            outcome(mesh_monotonic_elapsed(5, 3)).0,
            1,
            "finish precedes start"
        );
    }

    #[test]
    fn clock_never_moves_backwards() {
        let first = mesh_monotonic_now_nanos();
        let second = mesh_monotonic_now_nanos();
        assert!(second >= first);
    }
}
