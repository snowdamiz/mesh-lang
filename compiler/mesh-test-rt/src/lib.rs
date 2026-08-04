//! Runtime support that is linked exclusively into `meshc test` executables.
//!
//! This crate deliberately has a different archive name from `mesh-rt`. Its
//! static library bundles the production runtime and adds test-only fixtures,
//! so test programs link this archive *instead of* the production archive.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use mesh_rt::bytes::{mesh_bytes_copy_to, mesh_bytes_length, MeshBytes};
use mesh_rt::library::{
    mesh_library_init, mesh_library_register_host_callbacks, MeshLibraryHostCallbacksV1,
    MESH_LIBRARY_OK,
};
use parking_lot::Mutex;
use zeroize::Zeroizing;

const MAX_BOUNDARY_BYTES: usize = 1024 * 1024;
const MAX_KEY_BYTES: usize = 4096;
const MAX_ENTRIES: usize = 256;
const MAX_STORED_BYTES: usize = MAX_BOUNDARY_BYTES;
const INVALID_INPUT: i32 = 1;
const NOT_FOUND: i32 = 2;
const PLATFORM_FAILURE: i32 = 3;
const OUTPUT_TOO_LARGE: i32 = 4;

#[derive(Default)]
struct TestSecureStore {
    values: HashMap<Vec<u8>, Zeroizing<Vec<u8>>>,
    total_bytes: usize,
    installed: bool,
}

struct TestPushToken {
    selector: Vec<u8>,
    token: Zeroizing<Vec<u8>>,
}

static TEST_SECURE_STORE: OnceLock<Mutex<TestSecureStore>> = OnceLock::new();
static TEST_PUSH_TOKEN: OnceLock<Mutex<Option<TestPushToken>>> = OnceLock::new();

fn test_secure_store() -> &'static Mutex<TestSecureStore> {
    TEST_SECURE_STORE.get_or_init(|| Mutex::new(TestSecureStore::default()))
}

fn test_push_token() -> &'static Mutex<Option<TestPushToken>> {
    TEST_PUSH_TOKEN.get_or_init(|| Mutex::new(None))
}

unsafe extern "C" fn secure_store_put(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    _output: *mut u8,
    _output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    if output_len.is_null() {
        return INVALID_INPUT;
    }
    output_len.write(0);
    let Ok(input_len) = usize::try_from(input_len) else {
        return OUTPUT_TOO_LARGE;
    };
    if input.is_null() || !(5..=MAX_BOUNDARY_BYTES).contains(&input_len) {
        return INVALID_INPUT;
    }

    let input = std::slice::from_raw_parts(input, input_len);
    let key_len = u32::from_be_bytes(input[..4].try_into().expect("four-byte prefix")) as usize;
    if key_len == 0 || key_len > MAX_KEY_BYTES || key_len >= input.len() - 4 {
        return INVALID_INPUT;
    }
    let key = &input[4..4 + key_len];
    let value = &input[4 + key_len..];

    let mut store = test_secure_store().lock();
    if !store.installed {
        return PLATFORM_FAILURE;
    }
    let replaced_bytes = store.values.get(key).map_or(0, |old| key.len() + old.len());
    if replaced_bytes == 0 && store.values.len() == MAX_ENTRIES {
        return OUTPUT_TOO_LARGE;
    }
    let Some(total_bytes) = store
        .total_bytes
        .checked_sub(replaced_bytes)
        .and_then(|total| total.checked_add(key.len() + value.len()))
    else {
        return OUTPUT_TOO_LARGE;
    };
    if total_bytes > MAX_STORED_BYTES {
        return OUTPUT_TOO_LARGE;
    }

    store
        .values
        .insert(key.to_vec(), Zeroizing::new(value.to_vec()));
    store.total_bytes = total_bytes;
    MESH_LIBRARY_OK
}

unsafe extern "C" fn secure_store_get(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    output: *mut u8,
    output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    if output.is_null() || output_len.is_null() {
        return INVALID_INPUT;
    }
    output_len.write(0);
    let (Ok(input_len), Ok(output_capacity)) =
        (usize::try_from(input_len), usize::try_from(output_capacity))
    else {
        return OUTPUT_TOO_LARGE;
    };
    if input.is_null() || input_len == 0 || input_len > MAX_KEY_BYTES {
        return INVALID_INPUT;
    }

    let key = std::slice::from_raw_parts(input, input_len);
    let store = test_secure_store().lock();
    if !store.installed {
        return PLATFORM_FAILURE;
    }
    let Some(value) = store.values.get(key) else {
        return NOT_FOUND;
    };
    if value.len() > output_capacity {
        return OUTPUT_TOO_LARGE;
    }
    ptr::copy_nonoverlapping(value.as_ptr(), output, value.len());
    output_len.write(value.len() as u64);
    MESH_LIBRARY_OK
}

unsafe extern "C" fn secure_store_delete(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    _output: *mut u8,
    _output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    if output_len.is_null() {
        return INVALID_INPUT;
    }
    output_len.write(0);
    let Ok(input_len) = usize::try_from(input_len) else {
        return OUTPUT_TOO_LARGE;
    };
    if input.is_null() || input_len == 0 || input_len > MAX_KEY_BYTES {
        return INVALID_INPUT;
    }

    let key = std::slice::from_raw_parts(input, input_len);
    let mut store = test_secure_store().lock();
    if !store.installed {
        return PLATFORM_FAILURE;
    }
    if let Some(value) = store.values.remove(key) {
        store.total_bytes -= key.len() + value.len();
    }
    MESH_LIBRARY_OK
}

unsafe extern "C" fn push_get_token(
    _context: *mut c_void,
    input: *const u8,
    input_len: u64,
    output: *mut u8,
    output_capacity: u64,
    output_len: *mut u64,
) -> i32 {
    if output.is_null() || output_len.is_null() {
        return INVALID_INPUT;
    }
    output_len.write(0);
    let (Ok(input_len), Ok(output_capacity)) =
        (usize::try_from(input_len), usize::try_from(output_capacity))
    else {
        return OUTPUT_TOO_LARGE;
    };
    if input.is_null() || input_len == 0 || input_len > MAX_KEY_BYTES {
        return INVALID_INPUT;
    }

    let fixture = test_push_token().lock();
    let Some(fixture) = fixture.as_ref() else {
        return PLATFORM_FAILURE;
    };
    if std::slice::from_raw_parts(input, input_len) != fixture.selector {
        return INVALID_INPUT;
    }
    let token = &fixture.token;
    if token.len() > output_capacity {
        return OUTPUT_TOO_LARGE;
    }
    ptr::copy_nonoverlapping(token.as_ptr(), output, token.len());
    output_len.write(token.len() as u64);
    MESH_LIBRARY_OK
}

fn clear_secure_store() {
    let mut store = test_secure_store().lock();
    store.values.clear();
    store.total_bytes = 0;
    store.installed = false;
}

fn register_host_fixtures() -> i32 {
    let secure_store_installed = test_secure_store().lock().installed;
    let push_token_set = test_push_token().lock().is_some();
    let callbacks = MeshLibraryHostCallbacksV1 {
        secure_store_put: secure_store_installed.then_some(secure_store_put),
        secure_store_get: secure_store_installed.then_some(secure_store_get),
        secure_store_delete: secure_store_installed.then_some(secure_store_delete),
        push_get_token: push_token_set.then_some(push_get_token),
        ..MeshLibraryHostCallbacksV1::default()
    };
    mesh_library_register_host_callbacks(&callbacks)
}

fn reset_host_fixtures() {
    clear_secure_store();
    test_push_token().lock().take();

    // Keep the runtime initialized but remove all fixture callbacks between
    // test bodies. Registration remains lifecycle-checked by mesh-rt.
    let callbacks = MeshLibraryHostCallbacksV1::default();
    let _ = mesh_library_register_host_callbacks(&callbacks);
}

/// Install a bounded, zeroizing in-memory secure store for one Mesh test body.
#[no_mangle]
pub extern "C" fn mesh_test_install_in_memory_secure_store() -> u8 {
    clear_secure_store();
    if mesh_library_init() != MESH_LIBRARY_OK {
        return 0;
    }

    test_secure_store().lock().installed = true;
    if register_host_fixtures() != MESH_LIBRARY_OK {
        test_secure_store().lock().installed = false;
        return 0;
    }

    mesh_rt::test::register_test_case_cleanup_hook(reset_host_fixtures);
    1
}

/// Set the binary push token returned for one exact provider selector in a test body.
#[no_mangle]
pub extern "C" fn mesh_test_set_push_token(
    selector: *const MeshBytes,
    token: *const MeshBytes,
) -> u8 {
    if selector.is_null() || token.is_null() {
        return 0;
    }
    let (Ok(selector_len), Ok(token_len)) = (
        usize::try_from(mesh_bytes_length(selector)),
        usize::try_from(mesh_bytes_length(token)),
    ) else {
        return 0;
    };
    if selector_len == 0 || selector_len > MAX_KEY_BYTES || token_len > MAX_BOUNDARY_BYTES {
        return 0;
    }
    let mut selector_value = vec![0; selector_len];
    if mesh_bytes_copy_to(
        selector,
        0,
        selector_value.as_mut_ptr(),
        selector_len as u64,
    ) != selector_len as i64
    {
        return 0;
    }
    let mut value = Zeroizing::new(vec![0; token_len]);
    if mesh_bytes_copy_to(token, 0, value.as_mut_ptr(), token_len as u64) != token_len as i64 {
        return 0;
    }
    if mesh_library_init() != MESH_LIBRARY_OK {
        return 0;
    }

    *test_push_token().lock() = Some(TestPushToken {
        selector: selector_value,
        token: value,
    });
    if register_host_fixtures() != MESH_LIBRARY_OK {
        test_push_token().lock().take();
        return 0;
    }

    mesh_rt::test::register_test_case_cleanup_hook(reset_host_fixtures);
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_rt::library::{mesh_library_host_call, MeshLibraryHostCallback};
    use std::sync::Mutex as StdMutex;

    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn invoke(capability: u32, input: &[u8], output: &mut [u8]) -> (i32, usize) {
        let callback: MeshLibraryHostCallback = match capability {
            1 => secure_store_put,
            2 => secure_store_get,
            3 => secure_store_delete,
            4 => push_get_token,
            _ => panic!("unknown test capability"),
        };
        let mut output_len = 0;
        let status = unsafe {
            callback(
                ptr::null_mut(),
                input.as_ptr(),
                input.len() as u64,
                output.as_mut_ptr(),
                output.len() as u64,
                &mut output_len,
            )
        };
        (status, output_len as usize)
    }

    fn put_request(key: &[u8], value: &[u8]) -> Vec<u8> {
        let mut request = Vec::with_capacity(4 + key.len() + value.len());
        request.extend_from_slice(&(key.len() as u32).to_be_bytes());
        request.extend_from_slice(key);
        request.extend_from_slice(value);
        request
    }

    fn set_push_token(selector: &[u8], token: &[u8]) -> u8 {
        mesh_test_set_push_token(
            mesh_rt::bytes::mesh_bytes_new(selector.as_ptr(), selector.len() as u64),
            mesh_rt::bytes::mesh_bytes_new(token.as_ptr(), token.len() as u64),
        )
    }

    fn host_call_tag(capability: u32, input: &[u8]) -> u8 {
        let input = mesh_rt::bytes::mesh_bytes_new(input.as_ptr(), input.len() as u64);
        unsafe { (*mesh_library_host_call(capability, input)).tag }
    }

    fn assert_secure_store_and_push_callbacks() {
        assert_eq!(host_call_tag(3, b"key"), 0);
        assert_eq!(host_call_tag(4, b"expo/v1"), 0);
    }

    #[test]
    fn in_memory_secure_store_uses_platform_framing_and_statuses() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);

        let mut output = [0; 8];
        assert_eq!(
            invoke(1, &put_request(b"key", b"secret"), &mut output),
            (0, 0)
        );
        assert_eq!(invoke(2, b"key", &mut output), (0, 6));
        assert_eq!(&output[..6], b"secret");
        assert_eq!(invoke(2, b"missing", &mut output), (2, 0));
        assert_eq!(invoke(2, b"key", &mut output[..5]), (4, 0));
        assert_eq!(invoke(3, b"key", &mut output), (0, 0));
        assert_eq!(invoke(2, b"key", &mut output), (2, 0));
        assert_eq!(invoke(1, &[0, 0, 0, 0, 1], &mut output), (1, 0));

        reset_host_fixtures();
    }

    #[test]
    fn in_memory_secure_store_is_bounded_but_allows_replacement() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);
        let mut output = [];

        for index in 0..MAX_ENTRIES {
            let key = (index as u32).to_be_bytes();
            assert_eq!(invoke(1, &put_request(&key, b"x"), &mut output), (0, 0));
        }
        assert_eq!(
            invoke(1, &put_request(b"overflow", b"x"), &mut output),
            (4, 0)
        );
        assert_eq!(
            invoke(1, &put_request(&0u32.to_be_bytes(), b"new"), &mut output),
            (0, 0)
        );

        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);
        let large_value = vec![0; MAX_BOUNDARY_BYTES - 5];
        assert_eq!(
            invoke(1, &put_request(b"a", &large_value), &mut output),
            (0, 0)
        );
        assert_eq!(invoke(1, &put_request(b"b", b"four"), &mut output), (4, 0));
        reset_host_fixtures();
    }

    #[test]
    fn test_boundary_cleanup_disables_and_zeroizes_host_fixtures() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);
        assert_eq!(set_push_token(b"expo/v1", b"secret push token"), 1);
        let mut output = [];
        assert_eq!(
            invoke(1, &put_request(b"key", b"secret"), &mut output),
            (0, 0)
        );

        let test_name = b"next test";
        mesh_rt::test::mesh_test_begin(mesh_rt::string::mesh_string_new(
            test_name.as_ptr(),
            test_name.len() as u64,
        ));
        assert_eq!(
            invoke(1, &put_request(b"key", b"value"), &mut output),
            (3, 0)
        );
        assert_eq!(invoke(4, b"expo/v1", &mut [0; 32]), (3, 0));
        assert!(test_push_token().lock().is_none());
        assert!(!test_secure_store().lock().installed);
        assert_eq!(host_call_tag(3, b"key"), 1);
        assert_eq!(host_call_tag(4, b"expo/v1"), 1);
    }

    #[test]
    fn push_token_uses_host_framing_and_accepts_binary_values() {
        let _guard = TEST_LOCK.lock().unwrap();
        let token = [0, 0xff, 0x80, b'x'];
        let selector = mesh_rt::bytes::mesh_bytes_new(b"expo/raw/v1".as_ptr(), 11);
        let token = mesh_rt::bytes::mesh_bytes_new(token.as_ptr(), token.len() as u64);
        assert_eq!(mesh_test_set_push_token(selector, token), 1);

        let mut output = [0; 4];
        assert_eq!(invoke(4, b"expo/raw/v1", &mut output), (0, 4));
        assert_eq!(output, [0, 0xff, 0x80, b'x']);
        assert_eq!(invoke(4, b"other", &mut output), (1, 0));
        assert_eq!(invoke(4, b"expo/raw/v1", &mut output[..3]), (4, 0));

        assert_eq!(set_push_token(b"empty/v1", &[]), 1);
        assert_eq!(invoke(4, b"empty/v1", &mut []), (0, 0));
        let oversized = MeshBytes {
            len: (MAX_BOUNDARY_BYTES + 1) as u64,
        };
        assert_eq!(mesh_test_set_push_token(selector, &oversized), 0);
        let empty_selector = mesh_rt::bytes::mesh_bytes_new(b"".as_ptr(), 0);
        assert_eq!(mesh_test_set_push_token(empty_selector, token), 0);

        reset_host_fixtures();
    }

    #[test]
    fn secure_store_then_push_token_keeps_both_callbacks() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);
        assert_eq!(set_push_token(b"expo/v1", b"token"), 1);
        assert_secure_store_and_push_callbacks();
        reset_host_fixtures();
    }

    #[test]
    fn push_token_then_secure_store_keeps_both_callbacks() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert_eq!(set_push_token(b"expo/v1", b"token"), 1);
        assert_eq!(mesh_test_install_in_memory_secure_store(), 1);
        assert_secure_store_and_push_callbacks();
        reset_host_fixtures();
    }
}
