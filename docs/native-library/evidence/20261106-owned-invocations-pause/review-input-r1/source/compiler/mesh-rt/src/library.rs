//! Stable embedding boundary for static and dynamic Mesh libraries.
//!
//! Each invocation owns a short-lived runtime process. Only copied response
//! bytes cross the ABI; persistent state belongs in storage or spawned actors.

pub(crate) mod invocation;
pub(crate) use invocation::OwnedInvocation;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use parking_lot::{Mutex, RwLock};
use zeroize::{Zeroize, Zeroizing};

use crate::actor::{self, stack, ProcessId};
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::gc::mesh_rt_init;
use crate::io::{alloc_result, err_result, MeshResult};
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

// Callback buffers remain fixed and small independently of exported file payloads.
const MAX_BOUNDARY_BYTES: usize = 1024 * 1024;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;

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
    unsafe extern "C-unwind" fn(*mut MeshBytes, *mut MeshLibraryCallResult);
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
pub(crate) struct RegisteredCallbacks {
    pub(crate) context: usize,
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

impl From<&MeshLibraryHostCallbacksV1> for RegisteredCallbacks {
    fn from(callbacks: &MeshLibraryHostCallbacksV1) -> Self {
        Self {
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
        }
    }
}

pub(crate) fn valid_callbacks(callbacks: &MeshLibraryHostCallbacksV1) -> bool {
    callbacks.abi_version == MESH_LIBRARY_ABI_VERSION
        && callbacks.struct_size as usize == std::mem::size_of::<MeshLibraryHostCallbacksV1>()
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
    Stopping,
    Shutdown,
}

static LIFECYCLE: Mutex<Lifecycle> = Mutex::new(Lifecycle::New);
static CALL_LOCK: Mutex<()> = Mutex::new(());
static HOST_CALLBACKS: RwLock<Option<RegisteredCallbacks>> = RwLock::new(None);

struct CallContext {
    pid: ProcessId,
    previous_pid: Option<ProcessId>,
    // Dropped after CallContext::drop restores the PID and allocation cache.
    thread_state: CallThreadState,
}

/// A library call is synchronous, even when a native caller happens to be on
/// a Mesh coroutine stack. Never yield that outer coroutine with the call's
/// temporary PID installed. Its stack cannot use native-thread GC bounds.
struct CallThreadState {
    yielder: Option<*const ()>,
    reductions: u32,
    gc_wanted: bool,
}

impl CallThreadState {
    fn enter() -> Self {
        stack::CURRENT_YIELDER.with(|slot| Self {
            yielder: slot.yielder.replace(None),
            reductions: slot.reductions.get(),
            gc_wanted: slot.gc_wanted.replace(false),
        })
    }
}

impl Drop for CallThreadState {
    fn drop(&mut self) {
        stack::CURRENT_YIELDER.with(|slot| {
            slot.yielder.set(self.yielder);
            slot.reductions.set(self.reductions);
            slot.gc_wanted.set(self.gc_wanted);
        });
    }
}

impl Drop for CallContext {
    fn drop(&mut self) {
        if let Some(scheduler) = actor::GLOBAL_SCHEDULER.get() {
            scheduler.finalize_host_process(self.pid);
        }
        stack::clear_current_pid();
        if let Some(pid) = self.previous_pid {
            stack::set_current_pid(pid);
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_library_enable_protected_v2(required: u64) -> i32 {
    if crate::authority::in_owned_callback() {
        return MESH_LIBRARY_ERR_BUSY;
    }

    if required == 0 || required & !crate::authority::mesh_library_capabilities_v2() != 0 {
        return crate::authority::UNSUPPORTED;
    }
    let lifecycle = LIFECYCLE.lock();
    if !matches!(*lifecycle, Lifecycle::New) {
        return MESH_LIBRARY_ERR_BUSY;
    }
    crate::authority::activate();
    MESH_LIBRARY_OK
}

#[no_mangle]
pub extern "C" fn mesh_library_init() -> i32 {
    if crate::authority::in_owned_callback() {
        return MESH_LIBRARY_ERR_BUSY;
    }

    let mut lifecycle = LIFECYCLE.lock();
    match *lifecycle {
        Lifecycle::Running(_) => MESH_LIBRARY_OK,
        Lifecycle::Stopping | Lifecycle::Shutdown => MESH_LIBRARY_ERR_NOT_INITIALIZED,
        Lifecycle::New => {
            mesh_rt_init();
            actor::mesh_rt_init_actor(1);
            // The thread that starts the scheduler gets the main process; one
            // that finds it started gets a process of its own.
            let pid = stack::get_current_pid().unwrap_or_else(|| {
                let pid = actor::global_scheduler().create_main_process();
                stack::set_current_pid(pid);
                pid
            });
            *lifecycle = Lifecycle::Running(pid);
            MESH_LIBRARY_OK
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_library_shutdown() -> i32 {
    if crate::authority::in_callback() || crate::authority::current().is_some() {
        return MESH_LIBRARY_ERR_BUSY;
    }
    let _call = CALL_LOCK.lock();
    let pid = {
        let mut lifecycle = LIFECYCLE.lock();
        let Lifecycle::Running(pid) = *lifecycle else {
            return MESH_LIBRARY_OK;
        };
        *lifecycle = Lifecycle::Stopping;
        pid
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
    *LIFECYCLE.lock() = Lifecycle::Shutdown;
    MESH_LIBRARY_OK
}

#[no_mangle]
pub extern "C" fn mesh_library_register_host_callbacks(
    callbacks: *const MeshLibraryHostCallbacksV1,
) -> i32 {
    if crate::authority::in_owned_callback() {
        return MESH_LIBRARY_ERR_BUSY;
    }

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

    *HOST_CALLBACKS.write() = Some(RegisteredCallbacks::from(callbacks));
    MESH_LIBRARY_OK
}

pub(crate) fn secure_store_get_raw(input: &[u8], output: &mut [u8]) -> Result<usize, i32> {
    call_raw_host_callback(2, input, output)
}

pub(crate) fn secure_store_put_raw(input: &[u8]) -> Result<(), i32> {
    let mut ignored = [0u8; 1];
    call_raw_host_callback(1, input, &mut ignored).map(|_| ())
}

pub(crate) fn secure_store_delete_raw(input: &[u8]) -> Result<(), i32> {
    call_raw_host_callback(3, input, &mut []).map(|_| ())
}

#[derive(Debug)]
pub(crate) enum HostFailure {
    Missing,
    Callback(i32),
    OutputTooLarge,
    Refused,
}
impl HostFailure {
    fn status(self) -> i32 {
        match self {
            Self::Missing => MESH_LIBRARY_ERR_CALLBACK_MISSING,
            Self::Callback(status) => status,
            Self::OutputTooLarge => MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE,
            Self::Refused => crate::authority::REFUSED,
        }
    }
}
fn call_raw_host_callback(capability: u32, input: &[u8], output: &mut [u8]) -> Result<usize, i32> {
    call_host_callback(capability, input, output).map_err(HostFailure::status)
}

fn call_host_callback(
    capability: u32,
    input: &[u8],
    output: &mut [u8],
) -> Result<usize, HostFailure> {
    debug_assert!(input.len() <= MAX_BOUNDARY_BYTES && output.len() <= MAX_BOUNDARY_BYTES);
    let context = crate::authority::current();
    let callbacks = match &context {
        Some(context) => context.original.callbacks,
        None if crate::authority::protected() => return Err(HostFailure::Refused),
        None => *HOST_CALLBACKS.read(),
    }
    .ok_or(HostFailure::Missing)?;
    let callback = callbacks.callback(capability).ok_or(HostFailure::Missing)?;
    let call = HostCall {
        callbacks,
        callback,
        input,
        output,
    };
    if matches!(capability, 1 | 3) {
        crate::authority::perform(StoreMutation(call)).map_err(|_| HostFailure::Refused)?
    } else {
        crate::authority::perform(call).map_err(|_| HostFailure::Refused)?
    }
}

pub(crate) struct HostCall<'a> {
    callbacks: RegisteredCallbacks,
    callback: MeshLibraryHostCallback,
    input: &'a [u8],
    output: &'a mut [u8],
}
pub(crate) struct StoreMutation<'a>(HostCall<'a>);
impl crate::authority::FixedEffect for StoreMutation<'_> {
    type Output = Result<usize, HostFailure>;
    const LOCAL: bool = true;
    fn execute(self) -> Self::Output {
        crate::authority::FixedEffect::execute(self.0)
    }
}
impl crate::authority::FixedEffect for HostCall<'_> {
    type Output = Result<usize, HostFailure>;
    const LOCAL: bool = false;
    fn execute(self) -> Self::Output {
        crate::actor::stack::on_native_stack(move || {
            let _callback = crate::authority::callback_scope(crate::authority::current().is_some());
            let mut output_len = 0;
            let status = unsafe {
                (self.callback)(
                    self.callbacks.context as *mut c_void,
                    self.input.as_ptr(),
                    self.input.len() as u64,
                    self.output.as_mut_ptr(),
                    self.output.len() as u64,
                    &mut output_len,
                )
            };
            if status != MESH_LIBRARY_OK {
                return Err(HostFailure::Callback(status));
            }
            written_length(output_len, self.output.len()).ok_or(HostFailure::OutputTooLarge)
        })
    }
}

/// The length a host callback says it wrote, when it fits the `capacity`
/// it was given.
fn written_length(reported: u64, capacity: usize) -> Option<usize> {
    (reported <= capacity as u64).then_some(reported as usize)
}

/// Run an exported Mesh function on `input` and hand its result to `output`.
///
/// # Safety
///
/// `input` must point to `input_len` readable bytes (or be null with a
/// length of zero), and `output` to a writable `MeshLibraryBytes`.
#[no_mangle]
pub unsafe extern "C" fn mesh_library_invoke(
    entrypoint: MeshLibraryEntrypoint,
    input: *const u8,
    input_len: u64,
    output: *mut MeshLibraryBytes,
) -> i32 {
    mesh_library_invoke_bounded(
        entrypoint,
        input,
        input_len,
        output,
        MAX_BOUNDARY_BYTES as u64,
        MAX_BOUNDARY_BYTES as u64,
    )
}

/// Invoke with compiler-declared request and response/error limits. The original
/// entry point and generated host ABI retain their 1 MiB defaults and ownership.
/// Limits must be in 1..=64 MiB; they do not enlarge host callback buffers.
///
/// # Safety
/// Same pointer requirements as `mesh_library_invoke`.
#[no_mangle]
pub unsafe extern "C" fn mesh_library_invoke_bounded(
    entrypoint: MeshLibraryEntrypoint,
    input: *const u8,
    input_len: u64,
    output: *mut MeshLibraryBytes,
    max_input: u64,
    max_output: u64,
) -> i32 {
    if output.is_null() || (input.is_null() && input_len != 0) {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    (*output) = MeshLibraryBytes::default();
    if crate::authority::in_callback() {
        return MESH_LIBRARY_ERR_BUSY;
    }
    if !(1..=MAX_EXPORT_BYTES).contains(&max_input) || !(1..=MAX_EXPORT_BYTES).contains(&max_output)
    {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    if input_len > max_input {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    let invocation = crate::authority::current();
    if invocation.as_ref().is_some_and(|context| context.scheduled) {
        return MESH_LIBRARY_ERR_BUSY;
    }
    if crate::authority::protected() && invocation.as_ref().is_none_or(|context| !context.live()) {
        return crate::authority::REFUSED;
    }
    let Some(_call) = CALL_LOCK.try_lock() else {
        return MESH_LIBRARY_ERR_BUSY;
    };
    if !matches!(*LIFECYCLE.lock(), Lifecycle::Running(_)) {
        return MESH_LIBRARY_ERR_NOT_INITIALIZED;
    }
    // Running: mesh_library_init started the scheduler.
    let scheduler = actor::global_scheduler();

    // The byte ABI copies the result to caller-owned memory before this scope
    // ends. Reclaim the call's managed values and secrets on every exit path.
    let context = CallContext {
        pid: scheduler.create_main_process(),
        previous_pid: stack::get_current_pid(),
        thread_state: CallThreadState::enter(),
    };
    if let Some(process) = scheduler.get_process(context.pid) {
        let mut process = process.lock();
        process.library_call = true;
        process.invocation = invocation.clone();
        // Use the same safe-point collector as native-thread main, never the
        // allocator: runtime temporaries are not all visible GC roots. Calls
        // from a coroutine retain their heap until return, with its inherited
        // yielder isolated above; OS bounds do not describe coroutine stacks.
        if context.thread_state.yielder.is_none() {
            process.stack_base = stack::current_thread_stack_base();
            process.collects_at_safepoints = !process.stack_base.is_null();
        }
    }
    stack::set_current_pid(context.pid);
    let managed_input = mesh_bytes_new(input, input_len);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut result = MeshLibraryCallResult {
            tag: u8::MAX,
            _padding: [0; 7],
            value: ptr::null_mut(),
        };
        entrypoint(managed_input, &mut result);
        result
    }));
    if invocation.as_ref().is_some_and(|context| !context.live()) {
        return crate::authority::REFUSED;
    }
    match result {
        Ok(result) if result.tag == 0 => {
            copy_mesh_value(result.value, true, output, max_output, malloc)
        }
        Ok(result) if result.tag == 1 && invocation.is_some() => MESH_LIBRARY_ERR_APPLICATION,
        Ok(result) if result.tag == 1 => {
            let status = copy_mesh_value(result.value, false, output, max_output, malloc);
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

// Keep the actual allocation length private: a host can change the public
// response length before releasing it. The 16-byte prefix preserves native
// malloc alignment for these byte buffers on supported targets.
#[repr(C)]
struct ReturnedBytesHeader {
    len: usize,
    _padding: [u8; 16 - std::mem::size_of::<usize>()],
}

/// The C allocator paired only with `mesh_library_free_returned_bytes`.
unsafe fn malloc(len: usize) -> *mut u8 {
    let header_len = std::mem::size_of::<ReturnedBytesHeader>();
    let Some(total) = len
        .checked_add(header_len)
        .filter(|total| *total <= isize::MAX as usize)
    else {
        return ptr::null_mut();
    };
    if len == 0 {
        return ptr::null_mut();
    }
    let allocation = libc::malloc(total).cast::<ReturnedBytesHeader>();
    if allocation.is_null() {
        return ptr::null_mut();
    }
    allocation.write(ReturnedBytesHeader {
        len,
        _padding: [0; 16 - std::mem::size_of::<usize>()],
    });
    allocation.cast::<u8>().add(header_len)
}

/// Copy the Bytes (or, for an error, the String) an entry point returned into
/// `output`, in memory from `allocate`.
unsafe fn copy_mesh_value(
    value: *mut u8,
    bytes: bool,
    output: *mut MeshLibraryBytes,
    max_output: u64,
    allocate: unsafe fn(usize) -> *mut u8,
) -> i32 {
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
    if len > max_output {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    if len == 0 {
        return MESH_LIBRARY_OK;
    }
    let copied = allocate(len as usize);
    if copied.is_null() {
        return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
    }
    ptr::copy_nonoverlapping(data, copied, len as usize);
    (*output) = MeshLibraryBytes { data: copied, len };
    MESH_LIBRARY_OK
}

#[no_mangle]
pub extern "C" fn mesh_library_free_returned_bytes(bytes: *mut MeshLibraryBytes) {
    unsafe { free_returned_bytes_with(bytes, |data| libc::free(data)) }
}

unsafe fn free_returned_bytes_with(
    bytes: *mut MeshLibraryBytes,
    release: impl FnOnce(*mut c_void),
) {
    if bytes.is_null() {
        return;
    }
    if !(*bytes).data.is_null() {
        // Only pointers produced by this runtime's allocator are admissible.
        // Caller-controlled `len` never determines the erase or free extent.
        let header_len = std::mem::size_of::<ReturnedBytesHeader>();
        let allocation = (*bytes).data.sub(header_len);
        let len = allocation.cast::<ReturnedBytesHeader>().read().len;
        std::slice::from_raw_parts_mut(allocation, header_len + len).zeroize();
        release(allocation.cast());
    }
    *bytes = MeshLibraryBytes::default();
}

#[no_mangle]
pub extern "C" fn mesh_library_host_call(
    capability: u32,
    input: *const MeshBytes,
) -> *mut MeshResult {
    if !crate::authority::protected() && HOST_CALLBACKS.read().is_none() {
        return err_result("host_callback_not_registered");
    }
    if input.is_null() {
        return err_result("host_callback_input_missing");
    }
    let input = unsafe { (*input).as_slice() };
    if input.len() > MAX_BOUNDARY_BYTES {
        return err_result("host_callback_input_too_large");
    }
    let mut output = host_callback_output();
    match call_host_callback(capability, input, &mut output) {
        Ok(length) => alloc_result(0, mesh_bytes_new(output.as_ptr(), length as u64).cast()),
        Err(status) if crate::authority::protected() => {
            let _ = status;
            err_result(crate::authority::refuse())
        }
        Err(HostFailure::Missing) => err_result("host_callback_missing"),
        Err(HostFailure::OutputTooLarge) => err_result("host_callback_output_too_large"),
        Err(HostFailure::Callback(status)) => {
            err_result(&format!("host_callback_failed:{capability}:{status}"))
        }
        Err(HostFailure::Refused) => err_result("native_effect_refused"),
    }
}

fn host_callback_output() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(vec![0; MAX_BOUNDARY_BYTES])
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
    fn nested_thread_state_isolates_and_restores_gc_and_yield_notifications() {
        std::thread::spawn(|| {
            let outer = 1usize as *const ();
            stack::CURRENT_YIELDER.with(|slot| {
                slot.yielder.set(Some(outer));
                slot.reductions.set(17);
                slot.gc_wanted.set(true);
            });
            let result = catch_unwind(|| {
                let state = CallThreadState::enter();
                assert_eq!(state.yielder, Some(outer));
                stack::CURRENT_YIELDER.with(|slot| {
                    assert!(slot.yielder.get().is_none());
                    assert!(!slot.gc_wanted.get());
                    slot.reductions.set(3);
                    slot.gc_wanted.set(true);
                });
                panic!("test call unwinding");
            });
            assert!(result.is_err());
            stack::CURRENT_YIELDER.with(|slot| {
                assert_eq!(slot.yielder.get(), Some(outer));
                assert_eq!(slot.reductions.get(), 17);
                assert!(slot.gc_wanted.get());
                slot.yielder.set(None);
            });
            // The same guard restores a native thread's empty state too.
            {
                let _state = CallThreadState::enter();
                stack::CURRENT_YIELDER.with(|slot| slot.gc_wanted.set(false));
            }
            stack::CURRENT_YIELDER.with(|slot| assert!(slot.gc_wanted.get()));
            stack::CURRENT_YIELDER.with(|slot| slot.gc_wanted.set(false));
            {
                let _state = CallThreadState::enter();
                stack::CURRENT_YIELDER.with(|slot| slot.gc_wanted.set(true));
            }
            stack::CURRENT_YIELDER.with(|slot| assert!(!slot.gc_wanted.get()));
        })
        .join()
        .unwrap();
    }

    // Observe the bytes while they are still allocated, immediately before the
    // production release hook frees them. This never reads freed memory.
    unsafe fn release_and_observe_erasure(output: &mut MeshLibraryBytes, length: usize) -> bool {
        let payload = output.data;
        let mut erased = false;
        free_returned_bytes_with(output, |allocation| {
            let prefix = usize::try_from(payload.offset_from(allocation.cast::<u8>()))
                .expect("release receives the allocation base before the payload");
            erased = std::slice::from_raw_parts(allocation.cast::<u8>(), prefix + length)
                .iter()
                .all(|byte| *byte == 0);
            libc::free(allocation);
        });
        assert!(output.data.is_null());
        assert_eq!(output.len, 0);
        erased
    }

    #[test]
    fn returned_bytes_and_error_buffers_are_erased_before_release() {
        crate::gc::mesh_rt_init();
        let secret = b"private!";
        let values = [
            (
                mesh_bytes_new(secret.as_ptr(), secret.len() as u64).cast(),
                true,
            ),
            (
                crate::string::mesh_string_new(secret.as_ptr(), secret.len() as u64).cast(),
                false,
            ),
        ];
        let mut erased = Vec::new();
        for (value, bytes) in values {
            let mut output = MeshLibraryBytes::default();
            unsafe {
                assert_eq!(
                    copy_mesh_value(value, bytes, &mut output, MAX_EXPORT_BYTES, malloc),
                    MESH_LIBRARY_OK
                );
                assert_eq!(
                    std::slice::from_raw_parts(output.data, secret.len()),
                    secret
                );
                erased.push(release_and_observe_erasure(&mut output, secret.len()));
            }
        }
        assert_eq!(
            erased,
            [true, true],
            "both success and application-error copies must be erased"
        );
    }

    #[test]
    fn returned_wipe_uses_owned_length_not_public_length() {
        let secret = b"private!";
        let mut erased = Vec::new();
        for public_length in [0, u64::MAX] {
            unsafe {
                let payload = malloc(secret.len());
                assert!(!payload.is_null());
                ptr::copy_nonoverlapping(secret.as_ptr(), payload, secret.len());
                let mut output = MeshLibraryBytes {
                    data: payload,
                    len: public_length,
                };
                erased.push(release_and_observe_erasure(&mut output, secret.len()));
            }
        }
        assert_eq!(
            erased,
            [true, true],
            "caller-mutated length must not truncate or enlarge erasure"
        );
    }

    #[test]
    fn returned_empty_and_null_outputs_do_not_allocate() {
        unsafe fn unexpected_allocation(_len: usize) -> *mut u8 {
            panic!("empty returned values must not allocate");
        }
        crate::gc::mesh_rt_init();
        let values = [
            (mesh_bytes_new(ptr::null(), 0).cast(), true),
            (crate::string::mesh_string_new(ptr::null(), 0).cast(), false),
        ];
        for (value, bytes) in values {
            let mut output = MeshLibraryBytes::default();
            unsafe {
                assert_eq!(
                    copy_mesh_value(
                        value,
                        bytes,
                        &mut output,
                        MAX_EXPORT_BYTES,
                        unexpected_allocation
                    ),
                    MESH_LIBRARY_OK
                );
            }
            assert!(output.data.is_null());
            assert_eq!(output.len, 0);
            mesh_library_free_returned_bytes(&mut output);
            mesh_library_free_returned_bytes(&mut output);
        }
        mesh_library_free_returned_bytes(ptr::null_mut());
    }

    #[test]
    fn returned_allocation_alignment_and_overflow_are_bounded() {
        unsafe {
            let payload = malloc(1);
            assert!(!payload.is_null());
            payload.write(1);
            let aligned = payload as usize % std::mem::align_of::<u128>() == 0;
            let mut output = MeshLibraryBytes {
                data: payload,
                len: 1,
            };
            free_returned_bytes_with(&mut output, |allocation| libc::free(allocation));
            assert!(aligned);
            assert!(malloc(usize::MAX).is_null());
        }
    }

    /// A host whose allocator cannot give the returned bytes their memory
    /// gets an error, not a copy into nothing.
    #[test]
    fn returned_bytes_the_host_cannot_allocate_are_an_error() {
        unsafe fn out_of_memory(_len: usize) -> *mut u8 {
            ptr::null_mut()
        }
        crate::gc::mesh_rt_init();
        let value = mesh_bytes_new(b"x".as_ptr(), 1);
        let mut output = MeshLibraryBytes::default();
        let status = unsafe {
            copy_mesh_value(
                value.cast(),
                true,
                &mut output,
                MAX_BOUNDARY_BYTES as u64,
                out_of_memory,
            )
        };
        assert_eq!(status, MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
        assert!(output.data.is_null());
    }

    #[test]
    fn a_written_length_fits_its_capacity() {
        assert_eq!(written_length(3, 3), Some(3));
        assert_eq!(written_length(4, 3), None);
        assert_eq!(written_length(u64::MAX, usize::MAX), Some(usize::MAX));
    }

    #[test]
    fn host_callback_output_uses_zeroizing_boundary_buffer() {
        let output = host_callback_output();
        let _: &Zeroizing<Vec<u8>> = &output;

        assert_eq!(output.len(), MAX_BOUNDARY_BYTES);
    }
}
