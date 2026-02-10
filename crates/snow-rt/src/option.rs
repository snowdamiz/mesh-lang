//! Shared SnowOption struct and allocation helper for Snow's Option<T> representation.
//!
//! Used by env.rs, http/server.rs, collections/list.rs, and any other runtime
//! module that needs to return Option<T> to Snow programs.

use crate::gc::snow_gc_alloc_actor;

/// Tagged option value for Snow's Option<T> representation.
///
/// Layout matches the codegen layout for sum types:
/// - tag 0 = Some (first variant)
/// - tag 1 = None (second variant)
///
/// The value pointer points to the payload (e.g., a SnowString for Some,
/// null for None).
#[repr(C)]
pub struct SnowOption {
    pub tag: u8,
    pub value: *mut u8,
}

/// Allocate a SnowOption on the GC heap.
pub fn alloc_option(tag: u8, value: *mut u8) -> *mut SnowOption {
    unsafe {
        let ptr = snow_gc_alloc_actor(
            std::mem::size_of::<SnowOption>() as u64,
            std::mem::align_of::<SnowOption>() as u64,
        ) as *mut SnowOption;
        (*ptr).tag = tag;
        (*ptr).value = value;
        ptr
    }
}
