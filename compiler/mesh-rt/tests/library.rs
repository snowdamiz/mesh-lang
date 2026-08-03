use std::ffi::c_void;
use std::ptr;

use mesh_rt::bytes::{mesh_bytes_new, MeshBytes};
use mesh_rt::library::{
    mesh_library_free_returned_bytes, mesh_library_host_call, mesh_library_init,
    mesh_library_invoke, mesh_library_register_host_callbacks, mesh_library_shutdown,
    MeshLibraryBytes, MeshLibraryCallResult, MeshLibraryHostCallbacksV1, MESH_LIBRARY_ABI_VERSION,
    MESH_LIBRARY_ERR_ABI, MESH_LIBRARY_ERR_APPLICATION, MESH_LIBRARY_ERR_NOT_INITIALIZED,
    MESH_LIBRARY_ERR_PANIC, MESH_LIBRARY_OK,
};
use mesh_rt::string::mesh_string_new;

unsafe extern "C-unwind" fn echo(input: *mut MeshBytes) -> MeshLibraryCallResult {
    MeshLibraryCallResult {
        tag: 0,
        _padding: [0; 7],
        value: input.cast(),
    }
}

unsafe extern "C-unwind" fn reject(_input: *mut MeshBytes) -> MeshLibraryCallResult {
    let message = b"rejected";
    MeshLibraryCallResult {
        tag: 1,
        _padding: [0; 7],
        value: mesh_string_new(message.as_ptr(), message.len() as u64).cast(),
    }
}

unsafe extern "C-unwind" fn panic_entry(_input: *mut MeshBytes) -> MeshLibraryCallResult {
    panic!("contained")
}

unsafe extern "C" fn secure_store_get(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    output: *mut u8,
    output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    if input_len > output_capacity {
        return 1;
    }
    ptr::copy_nonoverlapping(input, output, input_len as usize);
    *output_len = input_len;
    0
}

#[test]
fn embedded_lifecycle_contains_failures_and_bounds_callback_ownership() {
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);

    let mut invalid_callbacks = MeshLibraryHostCallbacksV1::default();
    invalid_callbacks.abi_version = MESH_LIBRARY_ABI_VERSION + 1;
    assert_eq!(
        mesh_library_register_host_callbacks(&invalid_callbacks),
        MESH_LIBRARY_ERR_ABI
    );

    let callbacks = MeshLibraryHostCallbacksV1 {
        secure_store_get: Some(secure_store_get),
        ..MeshLibraryHostCallbacksV1::default()
    };
    assert_eq!(
        mesh_library_register_host_callbacks(&callbacks),
        MESH_LIBRARY_OK
    );

    let request = b"binary\0request";
    let managed = mesh_bytes_new(request.as_ptr(), request.len() as u64);
    let host_result = mesh_library_host_call(2, managed);
    assert_eq!(unsafe { (*host_result).tag }, 0);

    let mut output = MeshLibraryBytes::default();
    assert_eq!(
        unsafe { mesh_library_invoke(echo, request.as_ptr(), request.len() as u64, &mut output,) },
        MESH_LIBRARY_OK
    );
    assert_eq!(
        unsafe { std::slice::from_raw_parts(output.data, output.len as usize) },
        request
    );
    mesh_library_free_returned_bytes(&mut output);
    assert!(output.data.is_null());
    assert_eq!(output.len, 0);

    assert_eq!(
        unsafe { mesh_library_invoke(reject, ptr::null(), 0, &mut output) },
        MESH_LIBRARY_ERR_APPLICATION
    );
    mesh_library_free_returned_bytes(&mut output);
    assert_eq!(
        unsafe { mesh_library_invoke(panic_entry, ptr::null(), 0, &mut output) },
        MESH_LIBRARY_ERR_PANIC
    );

    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);
    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);
    assert_eq!(
        unsafe { mesh_library_invoke(echo, ptr::null(), 0, &mut output) },
        MESH_LIBRARY_ERR_NOT_INITIALIZED
    );
}
