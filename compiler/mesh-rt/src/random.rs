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

/// Raises a Mesh panic for a range with no values, so it unwinds.
#[no_mangle]
pub extern "C-unwind" fn mesh_random_next_int(state: i64, minimum: i64, maximum: i64) -> *mut u8 {
    if minimum > maximum {
        crate::panic::raise(format_args!(
            "Random.next_int: invalid range {minimum}..{maximum}"
        ));
    }
    // At most 2^64 values: the whole Int range.
    let span = (i128::from(maximum) - i128::from(minimum) + 1) as u128;
    let (next_state, random) = step(state as u64);
    let value = i128::from(minimum) + (u128::from(random) % span) as i128;
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

    /// An empty range is the program's error: a Mesh panic, which ends the
    /// actor alone.
    #[test]
    fn an_empty_range_raises_a_mesh_panic() {
        crate::gc::mesh_rt_init();
        let panic = std::panic::catch_unwind(|| mesh_random_next_int(1, 2, 1))
            .expect_err("an empty range was accepted");
        assert!(crate::panic::mesh_panic_message(&*panic).is_some());
    }

    /// The whole Int range is 2^64 values, one past what a u64 holds: every
    /// draw is a value of it.
    #[test]
    fn the_whole_int_range_can_be_drawn_from() {
        crate::gc::mesh_rt_init();
        let pair = mesh_random_next_int(42, i64::MIN, i64::MAX) as *const i64;
        let (state, value) = step(42);
        unsafe {
            assert_eq!(*pair.add(1), state as i64);
            assert_eq!(
                *pair.add(2),
                (i128::from(i64::MIN) + i128::from(value)) as i64
            );
        }
    }
}
