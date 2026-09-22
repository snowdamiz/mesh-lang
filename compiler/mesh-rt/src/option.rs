//! Shared MeshOption struct and allocation helper for Mesh's Option<T> representation.
//!
//! Used by env.rs, http/server.rs, collections/list.rs, and any other runtime
//! module that needs to return Option<T> to Mesh programs.

use crate::gc::mesh_gc_alloc_actor;

/// Tagged option value for Mesh's Option<T> representation.
///
/// Layout matches the codegen layout for sum types:
/// - tag 0 = Some (first variant)
/// - tag 1 = None (second variant)
///
/// The value pointer points to the payload (e.g., a MeshString for Some,
/// null for None).
#[repr(C)]
pub struct MeshOption {
    pub tag: u8,
    pub value: *mut u8,
}

/// Allocate a MeshOption on the GC heap.
/// Box the payload of a `Some` whose value is a scalar the runtime stored
/// raw (a list element, for example): compiled code reads a scalar payload
/// through a pointer, the way `Some(42)` and `String.to_int` store it.
#[no_mangle]
pub extern "C" fn mesh_option_box_scalar(option: *mut u8) -> *mut u8 {
    unsafe {
        let opt = option as *mut MeshOption;
        if !opt.is_null() && (*opt).tag == 0 {
            let boxed = mesh_gc_alloc_actor(8, 8) as *mut u64;
            *boxed = (*opt).value as u64;
            (*opt).value = boxed as *mut u8;
        }
        option
    }
}

pub fn alloc_option(tag: u8, value: *mut u8) -> *mut MeshOption {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshOption>() as u64,
            std::mem::align_of::<MeshOption>() as u64,
        ) as *mut MeshOption;
        (*ptr).tag = tag;
        (*ptr).value = value;
        ptr
    }
}
