use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use mesh_rt::bytes::{mesh_bytes_new, MeshBytes};
use mesh_rt::library::{
    mesh_library_free_returned_bytes, mesh_library_host_call, mesh_library_init,
    mesh_library_invoke, mesh_library_register_host_callbacks, mesh_library_shutdown,
    MeshLibraryBytes, MeshLibraryCallResult, MeshLibraryHostCallbacksV1, MESH_LIBRARY_ABI_VERSION,
    MESH_LIBRARY_ERR_ABI, MESH_LIBRARY_ERR_APPLICATION, MESH_LIBRARY_ERR_NOT_INITIALIZED,
    MESH_LIBRARY_ERR_PANIC, MESH_LIBRARY_OK,
};
use mesh_rt::string::mesh_string_new;
use mesh_rt::{
    mesh_storage_key_platform, mesh_storage_key_seal_bytes, mesh_storage_key_unseal_bytes,
    MeshSecretHandle,
};

fn secure_store() -> &'static Mutex<HashMap<Vec<u8>, Vec<u8>>> {
    static STORE: OnceLock<Mutex<HashMap<Vec<u8>, Vec<u8>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe extern "C" fn secure_store_put(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    _output: *mut u8,
    _output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    let input = std::slice::from_raw_parts(input, input_len as usize);
    if input.len() < 5 || output_len.is_null() {
        return 1;
    }
    let key_len = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
    if key_len == 0 || key_len >= input.len() - 4 {
        return 1;
    }
    secure_store().lock().unwrap().insert(
        input[4..4 + key_len].to_vec(),
        input[4 + key_len..].to_vec(),
    );
    output_len.write(0);
    0
}

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
    let input = std::slice::from_raw_parts(input, input_len as usize);
    if input == b"binary\0request" {
        if input_len > output_capacity {
            return 1;
        }
        ptr::copy_nonoverlapping(input.as_ptr(), output, input.len());
        *output_len = input_len;
        return 0;
    }
    let Some(value) = secure_store().lock().unwrap().get(input).cloned() else {
        return 2;
    };
    if value.len() as u64 > output_capacity {
        return 1;
    }
    ptr::copy_nonoverlapping(value.as_ptr(), output, value.len());
    *output_len = value.len() as u64;
    0
}

fn local_data_context() -> Vec<u8> {
    let mut context = vec![1];
    context.extend_from_slice(&[0x11; 32]);
    context.extend_from_slice(&[0x22; 16]);
    context.extend_from_slice(&[0x33; 32]);
    context.extend_from_slice(&[0x44; 32]);
    context.extend_from_slice(&14u16.to_be_bytes());
    context.extend_from_slice(&1u64.to_be_bytes());
    context
}

#[test]
fn embedded_lifecycle_contains_failures_and_bounds_callback_ownership() {
    secure_store().lock().unwrap().clear();
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);

    let mut invalid_callbacks = MeshLibraryHostCallbacksV1::default();
    invalid_callbacks.abi_version = MESH_LIBRARY_ABI_VERSION + 1;
    assert_eq!(
        mesh_library_register_host_callbacks(&invalid_callbacks),
        MESH_LIBRARY_ERR_ABI
    );

    let callbacks = MeshLibraryHostCallbacksV1 {
        secure_store_put: Some(secure_store_put),
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

    let platform_key = mesh_storage_key_platform();
    assert_eq!(unsafe { (*platform_key).tag }, 0);
    let platform_key = unsafe { (*platform_key).value.cast::<MeshSecretHandle>() };
    let plaintext = b"encrypted local message";
    let plaintext = mesh_bytes_new(plaintext.as_ptr(), plaintext.len() as u64);
    let context = local_data_context();
    let context = mesh_bytes_new(context.as_ptr(), context.len() as u64);
    let sealed = mesh_storage_key_seal_bytes(plaintext, platform_key, context);
    assert_eq!(unsafe { (*sealed).tag }, 0);
    let sealed = unsafe { (*sealed).value.cast::<MeshBytes>() };
    let opened = mesh_storage_key_unseal_bytes(sealed, platform_key, context);
    assert_eq!(unsafe { (*opened).tag }, 0);
    let opened = unsafe { &*(*opened).value.cast::<MeshBytes>() };
    let opened_bytes = unsafe {
        std::slice::from_raw_parts(
            (opened as *const MeshBytes)
                .cast::<u8>()
                .add(std::mem::size_of::<u64>()),
            opened.len as usize,
        )
    };
    assert_eq!(opened_bytes, b"encrypted local message");
    let store = secure_store().lock().unwrap();
    assert_eq!(store[b"mesh/storage-key/v1".as_slice()].len(), 36);
    assert_eq!(
        store[b"mesh/storage-counter/v1".as_slice()],
        1u64.to_be_bytes()
    );
    drop(store);

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
