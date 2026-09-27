//! Stable xorshift64* generator for deterministic replay and failure injection.

use crate::gc::mesh_gc_alloc_actor;

const ZERO_SEED: u64 = 0x9e37_79b9_7f4a_7c15;
const MULTIPLIER: u64 = 2_685_821_657_736_338_717;

fn step(mut state: u64) -> (u64, u64) {
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    (state, state.wrapping_mul(MULTIPLIER))
}

fn pair(state: u64, value: i64) -> *mut u8 {
    unsafe {
        let tuple = mesh_gc_alloc_actor(24, 8);
        *(tuple as *mut i64) = 2;
        *((tuple as *mut i64).add(1)) = state as i64;
        *((tuple as *mut i64).add(2)) = value;
        tuple
    }
}

#[no_mangle]
pub extern "C" fn mesh_random_seed(seed: i64) -> i64 {
    let state = seed as u64;
    if state == 0 {
        ZERO_SEED as i64
    } else {
        seed
    }
}

/// Raises a Mesh panic for a range with no values or more than a state can
/// pick from, so it unwinds.
#[no_mangle]
pub extern "C-unwind" fn mesh_random_next_int(state: i64, minimum: i64, maximum: i64) -> *mut u8 {
    if minimum > maximum {
        crate::panic::raise(format_args!(
            "Random.next_int: invalid range {minimum}..{maximum}"
        ));
    }
    let span = (i128::from(maximum) - i128::from(minimum) + 1) as u128;
    if span > u128::from(u64::MAX) {
        crate::panic::raise(format_args!("Random.next_int: range too wide"));
    }
    let (next_state, random) = step(state as u64);
    let value = i128::from(minimum) + i128::from(random % span as u64);
    pair(next_state, value as i64)
}

#[no_mangle]
pub extern "C" fn mesh_random_next_unit_ppm(state: i64) -> *mut u8 {
    mesh_random_next_int(state, 0, 999_999)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_has_a_stable_golden_value() {
        let (state, value) = step(42);
        assert_eq!(state, 1_409_286_176);
        assert_eq!(value % 100, 0);
    }

    /// A range with no values, or more values than a state can pick from,
    /// is the program's error: a Mesh panic, which ends the actor alone.
    #[test]
    fn an_empty_or_too_wide_range_raises_a_mesh_panic() {
        crate::gc::mesh_rt_init();
        for (minimum, maximum) in [(2, 1), (i64::MIN, i64::MAX)] {
            let panic = std::panic::catch_unwind(|| mesh_random_next_int(1, minimum, maximum))
                .expect_err("an unusable range was accepted");
            assert!(crate::panic::mesh_panic_message(&*panic).is_some());
        }
    }
}
