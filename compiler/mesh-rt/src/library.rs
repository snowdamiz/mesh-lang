//! Stable embedding boundary for static and dynamic Mesh libraries.

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use parking_lot::{Mutex, RwLock};
use zeroize::Zeroizing;

use crate::actor::{self, stack, ProcessId};
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::gc::mesh_rt_init;
use crate::io::{alloc_result, MeshResult};
use crate::string::MeshString;

pub const MESH_LIBRARY_ABI_VERSION: u32 = 1;
pub const MESH_LIBRARY_OK: i32 = 0;
pub const MESH_LIBRARY_ERR_INVALID_ARGUMENT: i32 = 1;
pub const MESH_LIBRARY_ERR_NOT_INITIALIZED: i32 = 2;
pub const MESH_LIBRARY_ERR_BUSY: i32 = 3;
pub const MESH_LIBRARY_ERR_PANIC: i32 = 4;
pub const MESH_LIBRARY_ERR_HOST_CALLBACK: i32 = 5;
pub const MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE: i32 = 6;
pub const MESH_LIBRARY_ERR_ABI: i32 = 7;
pub const MESH_LIBRARY_ERR_CALLBACK_MISSING: i32 = 8;
pub const MESH_LIBRARY_ERR_APPLICATION: i32 = 9;

// ponytail: one fixed 1 MiB host buffer; add negotiated sizing if mobile payloads exceed it.
const MAX_BOUNDARY_BYTES: usize = 1024 * 1024;

#[repr(C)]
#[derive(Debug, Default)]
pub struct MeshLibraryBytes {
    pub data: *mut u8,
    pub len: u64,
}

#[repr(C)]
pub struct MeshLibraryCallResult {
    pub tag: u8,
    pub _padding: [u8; 7],
    pub value: *mut u8,
}

pub type MeshLibraryEntrypoint =
    unsafe extern "C-unwind" fn(*mut MeshBytes) -> MeshLibraryCallResult;
pub type MeshLibraryHostCallback = unsafe extern "C" fn(
    context: *mut c_void,
    input: *const u8,
    input_len: u64,
    output: *mut u8,
    output_capacity: u64,
    output_len: *mut u64,
) -> i32;

#[repr(C)]
pub struct MeshLibraryHostCallbacksV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut c_void,
    pub secure_store_put: Option<MeshLibraryHostCallback>,
    pub secure_store_get: Option<MeshLibraryHostCallback>,
    pub secure_store_delete: Option<MeshLibraryHostCallback>,
    pub push_get_token: Option<MeshLibraryHostCallback>,
    pub background_schedule: Option<MeshLibraryHostCallback>,
    pub network_state: Option<MeshLibraryHostCallback>,
    pub monotonic_clock: Option<MeshLibraryHostCallback>,
    pub wall_clock: Option<MeshLibraryHostCallback>,
    pub log_redacted: Option<MeshLibraryHostCallback>,
}

impl Default for MeshLibraryHostCallbacksV1 {
    fn default() -> Self {
        Self {
            abi_version: MESH_LIBRARY_ABI_VERSION,
            struct_size: std::mem::size_of::<Self>() as u32,
            context: ptr::null_mut(),
            secure_store_put: None,
            secure_store_get: None,
            secure_store_delete: None,
            push_get_token: None,
            background_schedule: None,
            network_state: None,
            monotonic_clock: None,
            wall_clock: None,
            log_redacted: None,
        }
    }
}

#[derive(Clone, Copy)]
struct RegisteredCallbacks {
    context: usize,
    secure_store_put: Option<MeshLibraryHostCallback>,
    secure_store_get: Option<MeshLibraryHostCallback>,
    secure_store_delete: Option<MeshLibraryHostCallback>,
    push_get_token: Option<MeshLibraryHostCallback>,
    background_schedule: Option<MeshLibraryHostCallback>,
    network_state: Option<MeshLibraryHostCallback>,
    monotonic_clock: Option<MeshLibraryHostCallback>,
    wall_clock: Option<MeshLibraryHostCallback>,
    log_redacted: Option<MeshLibraryHostCallback>,
}

impl RegisteredCallbacks {
    fn callback(self, capability: u32) -> Option<MeshLibraryHostCallback> {
        match capability {
            1 => self.secure_store_put,
            2 => self.secure_store_get,
            3 => self.secure_store_delete,
            4 => self.push_get_token,
            5 => self.background_schedule,
            6 => self.network_state,
            7 => self.monotonic_clock,
            8 => self.wall_clock,
            9 => self.log_redacted,
            _ => None,
        }
    }
}

enum Lifecycle {
    New,
    Running(ProcessId),
    Shutdown,
}

static LIFECYCLE: Mutex<Lifecycle> = Mutex::new(Lifecycle::New);
static CALL_LOCK: Mutex<()> = Mutex::new(());
static HOST_CALLBACKS: RwLock<Option<RegisteredCallbacks>> = RwLock::new(None);

#[no_mangle]
pub extern "C" fn mesh_library_init() -> i32 {
    let mut lifecycle = LIFECYCLE.lock();
    match *lifecycle {
        Lifecycle::Running(_) => MESH_LIBRARY_OK,
        Lifecycle::Shutdown => MESH_LIBRARY_ERR_NOT_INITIALIZED,
        Lifecycle::New => {
            mesh_rt_init();
            actor::mesh_rt_init_actor(1);
            let Some(pid) = stack::get_current_pid() else {
                return MESH_LIBRARY_ERR_NOT_INITIALIZED;
            };
            *lifecycle = Lifecycle::Running(pid);
            MESH_LIBRARY_OK
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_library_shutdown() -> i32 {
    let _call = CALL_LOCK.lock();
    let mut lifecycle = LIFECYCLE.lock();
    let Lifecycle::Running(pid) = *lifecycle else {
        return MESH_LIBRARY_OK;
    };

    if let Some(scheduler) = actor::GLOBAL_SCHEDULER.get() {
        scheduler.finalize_host_process(pid);
        scheduler.signal_shutdown();
        scheduler.wait();
    }
    if stack::get_current_pid() == Some(pid) {
        stack::clear_current_pid();
    }
    *HOST_CALLBACKS.write() = None;
    *lifecycle = Lifecycle::Shutdown;
    MESH_LIBRARY_OK
}

#[no_mangle]
pub extern "C" fn mesh_library_register_host_callbacks(
    callbacks: *const MeshLibraryHostCallbacksV1,
) -> i32 {
    if callbacks.is_null() {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    let callbacks = unsafe { &*callbacks };
    if callbacks.abi_version != MESH_LIBRARY_ABI_VERSION
        || callbacks.struct_size as usize != std::mem::size_of::<MeshLibraryHostCallbacksV1>()
    {
        return MESH_LIBRARY_ERR_ABI;
    }
    if !matches!(*LIFECYCLE.lock(), Lifecycle::Running(_)) {
        return MESH_LIBRARY_ERR_NOT_INITIALIZED;
    }

    *HOST_CALLBACKS.write() = Some(RegisteredCallbacks {
        context: callbacks.context as usize,
        secure_store_put: callbacks.secure_store_put,
        secure_store_get: callbacks.secure_store_get,
        secure_store_delete: callbacks.secure_store_delete,
        push_get_token: callbacks.push_get_token,
        background_schedule: callbacks.background_schedule,
        network_state: callbacks.network_state,
        monotonic_clock: callbacks.monotonic_clock,
        wall_clock: callbacks.wall_clock,
        log_redacted: callbacks.log_redacted,
    });
    MESH_LIBRARY_OK
}

pub(crate) fn secure_store_get_raw(input: &[u8], output: &mut [u8]) -> Result<usize, i32> {
    call_raw_host_callback(2, input, output)
}

pub(crate) fn secure_store_put_raw(input: &[u8]) -> Result<(), i32> {
    let mut ignored = [0u8; 1];
    call_raw_host_callback(1, input, &mut ignored).map(|_| ())
}

fn call_raw_host_callback(capability: u32, input: &[u8], output: &mut [u8]) -> Result<usize, i32> {
    if input.len() > MAX_BOUNDARY_BYTES || output.len() > MAX_BOUNDARY_BYTES {
        return Err(MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    }
    let callbacks = (*HOST_CALLBACKS.read()).ok_or(MESH_LIBRARY_ERR_NOT_INITIALIZED)?;
    let callback = callbacks
        .callback(capability)
        .ok_or(MESH_LIBRARY_ERR_CALLBACK_MISSING)?;
    let mut output_len = 0u64;
    let status = unsafe {
        callback(
            callbacks.context as *mut c_void,
            input.as_ptr(),
            input.len() as u64,
            output.as_mut_ptr(),
            output.len() as u64,
            &mut output_len,
        )
    };
    if status != MESH_LIBRARY_OK {
        return Err(status);
    }
    let output_len = usize::try_from(output_len).map_err(|_| MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE)?;
    if output_len > output.len() {
        return Err(MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    }
    Ok(output_len)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_library_invoke(
    entrypoint: MeshLibraryEntrypoint,
    input: *const u8,
    input_len: u64,
    output: *mut MeshLibraryBytes,
) -> i32 {
    if output.is_null() || (input.is_null() && input_len != 0) {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    (*output) = MeshLibraryBytes::default();
    let Ok(input_len) = usize::try_from(input_len) else {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    };
    if input_len > MAX_BOUNDARY_BYTES {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    let Some(_call) = CALL_LOCK.try_lock() else {
        return MESH_LIBRARY_ERR_BUSY;
    };
    let pid = match *LIFECYCLE.lock() {
        Lifecycle::Running(pid) => pid,
        Lifecycle::New | Lifecycle::Shutdown => return MESH_LIBRARY_ERR_NOT_INITIALIZED,
    };

    let previous_pid = stack::get_current_pid();
    stack::set_current_pid(pid);
    let managed_input = mesh_bytes_new(input, input_len as u64);
    let result = catch_unwind(AssertUnwindSafe(|| entrypoint(managed_input)));
    stack::clear_current_pid();
    if let Some(previous_pid) = previous_pid {
        stack::set_current_pid(previous_pid);
    }

    match result {
        Ok(result) if result.tag == 0 => copy_mesh_value(result.value, true, output),
        Ok(result) if result.tag == 1 => {
            let status = copy_mesh_value(result.value, false, output);
            if status == MESH_LIBRARY_OK {
                MESH_LIBRARY_ERR_APPLICATION
            } else {
                status
            }
        }
        Ok(_) => MESH_LIBRARY_ERR_INVALID_ARGUMENT,
        Err(_) => MESH_LIBRARY_ERR_PANIC,
    }
}

unsafe fn copy_mesh_value(value: *mut u8, bytes: bool, output: *mut MeshLibraryBytes) -> i32 {
    if value.is_null() {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    let (data, len) = if bytes {
        let value = &*value.cast::<MeshBytes>();
        (value.as_slice().as_ptr(), value.len)
    } else {
        let value = &*value.cast::<MeshString>();
        (value.data_ptr(), value.len)
    };
    let Ok(len_usize) = usize::try_from(len) else {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    };
    if len_usize > MAX_BOUNDARY_BYTES {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    if len_usize == 0 {
        return MESH_LIBRARY_OK;
    }
    let copied = libc::malloc(len_usize).cast::<u8>();
    if copied.is_null() {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    ptr::copy_nonoverlapping(data, copied, len_usize);
    (*output) = MeshLibraryBytes { data: copied, len };
    MESH_LIBRARY_OK
}

#[no_mangle]
pub extern "C" fn mesh_library_free_returned_bytes(bytes: *mut MeshLibraryBytes) {
    if bytes.is_null() {
        return;
    }
    unsafe {
        if !(*bytes).data.is_null() {
            libc::free((*bytes).data.cast());
        }
        *bytes = MeshLibraryBytes::default();
    }
}

#[no_mangle]
pub extern "C" fn mesh_library_host_call(
    capability: u32,
    input: *const MeshBytes,
) -> *mut MeshResult {
    if input.is_null() {
        return error_result("host_callback_invalid_input");
    }
    let callbacks = *HOST_CALLBACKS.read();
    let Some(callbacks) = callbacks else {
        return error_result("host_callback_not_registered");
    };
    let Some(callback) = callbacks.callback(capability) else {
        return error_result("host_callback_missing");
    };
    let input = unsafe { (*input).as_slice() };
    if input.len() > MAX_BOUNDARY_BYTES {
        return error_result("host_callback_input_too_large");
    }

    let mut output = host_callback_output();
    let mut output_len = 0u64;
    let status = unsafe {
        callback(
            callbacks.context as *mut c_void,
            input.as_ptr(),
            input.len() as u64,
            output.as_mut_ptr(),
            output.len() as u64,
            &mut output_len,
        )
    };
    if status != MESH_LIBRARY_OK {
        return error_result(&format!("host_callback_failed:{capability}:{status}"));
    }
    let Ok(output_len) = usize::try_from(output_len) else {
        return error_result("host_callback_output_too_large");
    };
    if output_len > output.len() {
        return error_result("host_callback_output_too_large");
    }
    alloc_result(0, mesh_bytes_new(output.as_ptr(), output_len as u64).cast())
}

fn host_callback_output() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(vec![0; MAX_BOUNDARY_BYTES])
}

fn error_result(message: &str) -> *mut MeshResult {
    alloc_result(
        1,
        crate::string::mesh_string_new(message.as_ptr(), message.len() as u64).cast(),
    )
}

macro_rules! host_entrypoint {
    ($name:ident, $capability:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(input: *const MeshBytes) -> *mut MeshResult {
            mesh_library_host_call($capability, input)
        }
    };
}

host_entrypoint!(mesh_host_secure_store_put, 1);
host_entrypoint!(mesh_host_secure_store_get, 2);
host_entrypoint!(mesh_host_secure_store_delete, 3);
host_entrypoint!(mesh_host_push_get_token, 4);
host_entrypoint!(mesh_host_background_schedule, 5);
host_entrypoint!(mesh_host_network_state, 6);
host_entrypoint!(mesh_host_monotonic_clock, 7);
host_entrypoint!(mesh_host_wall_clock, 8);
host_entrypoint!(mesh_host_log_redacted, 9);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_callback_output_uses_zeroizing_boundary_buffer() {
        let output = host_callback_output();
        let _: &Zeroizing<Vec<u8>> = &output;

        assert_eq!(output.len(), MAX_BOUNDARY_BYTES);
    }
}
