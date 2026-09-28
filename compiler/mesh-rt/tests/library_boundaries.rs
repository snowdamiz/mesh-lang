//! The embedding ABI's refusals, each at the boundary a host crosses. One
//! test: the library's lifecycle is process-wide.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use mesh_rt::bytes::{mesh_bytes_new, MeshBytes};
use mesh_rt::io::MeshResult;
use mesh_rt::library::*;
use mesh_rt::string::{mesh_string_new, MeshString};

const MIB: usize = 1024 * 1024;

/// How the host callback answers: 0 as asked, 1 with a status of 7, 2 with
/// a longer output than it was given room for.
static CALLBACK_MODE: AtomicU8 = AtomicU8::new(0);

unsafe extern "C" fn callback(
    _context: *mut c_void,
    _input: *const u8,
    _input_len: u64,
    _output: *mut u8,
    output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    match CALLBACK_MODE.load(Ordering::SeqCst) {
        0 => {
            output_len.write(0);
            0
        }
        1 => 7,
        _ => {
            output_len.write(output_capacity + 1);
            0
        }
    }
}

fn result_text(result: *mut MeshResult) -> Result<(), String> {
    let result = unsafe { &*result };
    match result.tag {
        0 => Ok(()),
        _ => Err(unsafe { (*(result.value as *const MeshString)).as_str() }.to_string()),
    }
}

fn host_call(capability: u32, input: &[u8]) -> Result<(), String> {
    let input = mesh_bytes_new(input.as_ptr(), input.len() as u64);
    result_text(mesh_library_host_call(capability, input))
}

fn returning(tag: u8, value: *mut u8, output: *mut MeshLibraryCallResult) {
    unsafe {
        output.write(MeshLibraryCallResult {
            tag,
            _padding: [0; 7],
            value,
        })
    };
}

unsafe extern "C-unwind" fn echo(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    returning(0, input.cast(), output);
}

unsafe extern "C-unwind" fn silent(_: *mut MeshBytes, _: *mut MeshLibraryCallResult) {}

unsafe extern "C-unwind" fn null_value(_: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    returning(0, ptr::null_mut(), output);
}

unsafe extern "C-unwind" fn huge(_: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    let bytes = vec![0u8; 2 * MIB];
    returning(
        0,
        mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64).cast(),
        output,
    );
}

unsafe extern "C-unwind" fn huge_error(_: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    let text = vec![b'e'; 2 * MIB];
    returning(
        1,
        mesh_string_new(text.as_ptr(), text.len() as u64).cast(),
        output,
    );
}

static BLOCKING: AtomicBool = AtomicBool::new(false);
static RELEASED: AtomicBool = AtomicBool::new(false);

unsafe extern "C-unwind" fn blocking(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    BLOCKING.store(true, Ordering::SeqCst);
    while !RELEASED.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    returning(0, input.cast(), output);
}

fn invoke(entry: MeshLibraryEntrypoint, input: &[u8]) -> (i32, Vec<u8>) {
    let mut output = MeshLibraryBytes::default();
    let status =
        unsafe { mesh_library_invoke(entry, input.as_ptr(), input.len() as u64, &mut output) };
    let bytes = if output.data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.data, output.len as usize) }.to_vec()
    };
    mesh_library_free_returned_bytes(&mut output);
    (status, bytes)
}

#[test]
fn the_embedding_abi_refuses_what_it_cannot_do() {
    // Before startup: no callbacks, and nothing to register them with.
    mesh_rt::gc::mesh_rt_init();
    assert_eq!(
        host_call(2, b"k"),
        Err("host_callback_not_registered".into())
    );
    assert_eq!(
        mesh_library_register_host_callbacks(ptr::null()),
        MESH_LIBRARY_ERR_INVALID_ARGUMENT
    );
    let every = Some(callback as MeshLibraryHostCallback);
    let callbacks = MeshLibraryHostCallbacksV1 {
        secure_store_put: every,
        secure_store_get: every,
        secure_store_delete: every,
        push_get_token: every,
        background_schedule: every,
        network_state: every,
        monotonic_clock: every,
        wall_clock: every,
        log_redacted: every,
        ..Default::default()
    };
    assert_eq!(
        mesh_library_register_host_callbacks(&callbacks),
        MESH_LIBRARY_ERR_NOT_INITIALIZED
    );
    mesh_library_free_returned_bytes(ptr::null_mut());

    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    assert_eq!(
        mesh_library_register_host_callbacks(&callbacks),
        MESH_LIBRARY_OK
    );

    // Every capability reaches its callback; one past them is missing.
    for capability in 1..=9 {
        assert_eq!(host_call(capability, b"k"), Ok(()), "{capability}");
    }
    assert_eq!(host_call(10, b"k"), Err("host_callback_missing".into()));
    assert_eq!(
        host_call(6, &vec![0; 2 * MIB]),
        Err("host_callback_input_too_large".into())
    );
    CALLBACK_MODE.store(1, Ordering::SeqCst);
    assert_eq!(host_call(6, b"k"), Err("host_callback_failed:6:7".into()));
    CALLBACK_MODE.store(2, Ordering::SeqCst);
    assert_eq!(
        host_call(6, b"k"),
        Err("host_callback_output_too_large".into())
    );
    // The storage key reads its record through a raw callback too.
    let platform = mesh_rt::mesh_storage_key_platform();
    assert_eq!(
        unsafe { (*platform).tag },
        1,
        "a record longer than asked for"
    );
    CALLBACK_MODE.store(0, Ordering::SeqCst);

    // An invocation's arguments and its entry point's answers.
    let mut output = MeshLibraryBytes::default();
    let refused = |status| assert_eq!(status, MESH_LIBRARY_ERR_INVALID_ARGUMENT);
    refused(unsafe { mesh_library_invoke(echo, ptr::null(), 0, ptr::null_mut()) });
    refused(unsafe { mesh_library_invoke(echo, ptr::null(), 1, &mut output) });
    let too_long = [0u8; 1];
    let status =
        unsafe { mesh_library_invoke(echo, too_long.as_ptr(), 2 * MIB as u64, &mut output) };
    assert_eq!(status, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    assert_eq!(invoke(silent, b"x").0, MESH_LIBRARY_ERR_INVALID_ARGUMENT);
    assert_eq!(
        invoke(null_value, b"x").0,
        MESH_LIBRARY_ERR_INVALID_ARGUMENT
    );
    assert_eq!(invoke(echo, b""), (MESH_LIBRARY_OK, Vec::new()));
    assert_eq!(invoke(huge, b"x").0, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    assert_eq!(
        invoke(huge_error, b"x").0,
        MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE
    );

    // One invocation at a time.
    let first = std::thread::spawn(|| invoke(blocking, b"first"));
    while !BLOCKING.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    assert_eq!(invoke(echo, b"second").0, MESH_LIBRARY_ERR_BUSY);
    RELEASED.store(true, Ordering::SeqCst);
    assert_eq!(first.join().unwrap(), (MESH_LIBRARY_OK, b"first".to_vec()));

    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);
    assert_eq!(mesh_library_init(), MESH_LIBRARY_ERR_NOT_INITIALIZED);
}
