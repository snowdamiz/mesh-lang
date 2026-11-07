//! Native-only, synchronous effect admission. No scheduler or publication grant.
//!
//! Generation tombstones only move false -> true. An authority retains at most
//! MAX_ANCESTRY immutable nodes in one domain; the domain has no lifetime quota.
//! A successful scan linearizes at its FIRST load: every later node observed
//! live was also live then. A retirement before that point is seen; one after
//! it may lose to this exact already-owned operation. There is no current-owner
//! lookup, retry, reusable permit, callback, allocation or resource lock in the
//! scan. Refused scans execute nothing. This monotone snapshot is invalid for
//! revivable nodes; consequently there is deliberately no reset/rebind API.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::Arc;

use crate::library::{MeshLibraryBytes, MeshLibraryHostCallbacksV1, RegisteredCallbacks};

pub const REFUSED: i32 = 10;
pub const UNSUPPORTED: i32 = 11;
pub const CAP_SYNCHRONOUS_SINKS: u64 = 1;
// Intentionally unadvertised: full effect inventory, scheduling, native policy,
// service-message provenance and actual private publication are separate gates.
const MAX_ANCESTRY: usize = 32;
static PROTECTED: AtomicBool = AtomicBool::new(false);
static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(1);

pub struct NativeDomain {
    _private: (),
}
pub struct NativeGeneration {
    domain: Arc<NativeDomain>,
    retired: AtomicBool,
    local_inflight: AtomicU64,
}

pub type NativeLifetime = unsafe extern "C" fn(*mut c_void);
#[repr(C)]
pub struct MeshLibraryHostCallbacksV2 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub callbacks: MeshLibraryHostCallbacksV1,
    pub retain: Option<NativeLifetime>,
    pub release: Option<NativeLifetime>,
}

pub struct NativeAuthority {
    nodes: Vec<Arc<NativeGeneration>>,
    pub(crate) callbacks: Option<RegisteredCallbacks>,
    release: Option<NativeLifetime>,
}
impl Drop for NativeAuthority {
    fn drop(&mut self) {
        if let (Some(callbacks), Some(release)) = (&self.callbacks, self.release) {
            unsafe { release(callbacks.context as *mut c_void) };
        }
    }
}
impl NativeAuthority {
    fn live(&self) -> bool {
        self.nodes.iter().all(|node| !node.retired.load(SeqCst))
    }
}

pub(crate) struct InvocationContext {
    pub(crate) original: Arc<NativeAuthority>,
    pub(crate) id: u64,
    sequence: AtomicU64,
    refused: AtomicBool,
    pub(crate) transactions: AtomicU64,
}
impl InvocationContext {
    pub(crate) fn live(&self) -> bool {
        !self.refused.load(SeqCst) && self.original.live()
    }
}
thread_local! {
    static CURRENT: RefCell<Option<Arc<InvocationContext>>> = const { RefCell::new(None) };
}
pub(crate) fn current() -> Option<Arc<InvocationContext>> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub(crate) struct ContextGuard(Option<Arc<InvocationContext>>);
pub(crate) fn enter(context: Option<Arc<InvocationContext>>) -> ContextGuard {
    ContextGuard(CURRENT.with(|slot| slot.replace(context)))
}
impl Drop for ContextGuard {
    fn drop(&mut self) {
        let removed = CURRENT.with(|slot| slot.replace(self.0.take()));
        drop(removed);
    }
}
pub(crate) fn protected() -> bool {
    PROTECTED.load(SeqCst) || current().is_some()
}
pub(crate) fn refuse() -> &'static str {
    if let Some(context) = current() {
        context.refused.store(true, SeqCst);
    }
    "native_effect_refused"
}
pub(crate) fn may_suspend() -> bool {
    current().is_none_or(|context| context.transactions.load(SeqCst) == 0)
}

/// Implemented only by concrete runtime sinks with fixed operands and exclusive
/// resource ownership. Never implement for a user/Mesh closure or a transaction
/// callback. The only outward result is the operation's completed outcome.
mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::http::client::HttpAttempt<'_> {}
    impl Sealed for crate::db::sqlite::SqlStatement {}
    impl Sealed for crate::db::sqlite::SqlPrepare<'_> {}
    impl Sealed for crate::db::sqlite::SqlOpen {}
    impl Sealed for crate::library::HostCall<'_> {}
    impl Sealed for crate::library::StoreMutation<'_> {}
}
pub(crate) trait FixedEffect: sealed::Sealed {
    type Output;
    const LOCAL: bool;
    fn execute(self) -> Self::Output;
}
struct LocalSettlement<'a>(&'a NativeAuthority);
impl Drop for LocalSettlement<'_> {
    fn drop(&mut self) {
        for node in &self.0.nodes {
            node.local_inflight.fetch_sub(1, SeqCst);
        }
    }
}
pub(crate) fn perform<E: FixedEffect>(effect: E) -> Result<E::Output, &'static str> {
    let Some(context) = current() else {
        return if PROTECTED.load(SeqCst) {
            Err(refuse())
        } else {
            Ok(effect.execute())
        };
    };
    // Register finite local work before ANY liveness load. A retire-then-drain
    // observer cannot miss an accepted writer. Failed admissions settle too.
    let settlement = E::LOCAL.then(|| {
        for node in &context.original.nodes {
            node.local_inflight.fetch_add(1, SeqCst);
        }
        LocalSettlement(&context.original)
    });
    // Sequence exhaustion refuses forever; wrapping could reuse effect identity.
    let sequence = context.sequence.load(SeqCst);
    let sequence = sequence.checked_add(1).ok_or(()).and_then(|next| {
        context
            .sequence
            .compare_exchange(sequence, next, SeqCst, SeqCst)
            .map_err(|_| ())
    });
    if sequence.is_err() || !context.live() {
        return Err(refuse());
    }
    let result = effect.execute();
    drop(settlement);
    Ok(result)
}

#[no_mangle]
pub extern "C" fn mesh_library_capabilities_v2() -> u64 {
    CAP_SYNCHRONOUS_SINKS
}
pub(crate) fn activate() {
    PROTECTED.store(true, SeqCst);
}

#[no_mangle]
pub extern "C" fn mesh_library_current_invocation_v2() -> u64 {
    current().map_or(0, |context| context.id)
}

/// All opaque pointers below are native-only handles. Callers keep a reference
/// alive for the entire call, do not forge pointers, and release each exactly
/// once. Concurrent retirement is supported; concurrent release of the same
/// reference is not. Mesh/JS/request bytes never supply these handles.
#[no_mangle]
pub unsafe extern "C" fn mesh_authority_domain_create_v2(out: *mut *const NativeDomain) -> i32 {
    if out.is_null() {
        return 1;
    }
    out.write(Arc::into_raw(Arc::new(NativeDomain { _private: () })));
    0
}
#[no_mangle]
pub unsafe extern "C" fn mesh_authority_generation_v2(
    domain: *const NativeDomain,
    out: *mut *const NativeGeneration,
) -> i32 {
    if domain.is_null() || out.is_null() {
        return 1;
    }
    Arc::increment_strong_count(domain);
    out.write(Arc::into_raw(Arc::new(NativeGeneration {
        domain: Arc::from_raw(domain),
        retired: AtomicBool::new(false),
        local_inflight: AtomicU64::new(0),
    })));
    0
}
#[no_mangle]
pub unsafe extern "C" fn mesh_authority_bind_v2(
    domain: *const NativeDomain,
    nodes: *const *const NativeGeneration,
    count: u64,
    callbacks: *const MeshLibraryHostCallbacksV2,
    out: *mut *const NativeAuthority,
) -> i32 {
    if out.is_null() {
        return 1;
    }
    out.write(std::ptr::null());
    if domain.is_null() || nodes.is_null() || count == 0 || count > MAX_ANCESTRY as u64 {
        return 1;
    }
    let mut owned = Vec::with_capacity(count as usize);
    for &node in std::slice::from_raw_parts(nodes, count as usize) {
        if node.is_null() || Arc::as_ptr(&(*node).domain) != domain || (*node).retired.load(SeqCst)
        {
            return REFUSED;
        }
        Arc::increment_strong_count(node);
        owned.push(Arc::from_raw(node));
    }
    let (registered, release) = if let Some(callbacks) = callbacks.as_ref() {
        if callbacks.abi_version != 2
            || callbacks.struct_size as usize != std::mem::size_of::<MeshLibraryHostCallbacksV2>()
            || !crate::library::valid_callbacks(&callbacks.callbacks)
        {
            return 7;
        }
        let (Some(retain), Some(release)) = (callbacks.retain, callbacks.release) else {
            return 1;
        };
        retain(callbacks.callbacks.context);
        (
            Some(RegisteredCallbacks::from(&callbacks.callbacks)),
            Some(release),
        )
    } else {
        (None, None)
    };
    out.write(Arc::into_raw(Arc::new(NativeAuthority {
        nodes: owned,
        callbacks: registered,
        release,
    })));
    0
}
#[no_mangle]
pub unsafe extern "C" fn mesh_authority_retire_v2(node: *const NativeGeneration) -> i32 {
    let Some(node) = node.as_ref() else {
        return 1;
    };
    node.retired.store(true, SeqCst);
    0
}
#[no_mangle]
pub unsafe extern "C" fn mesh_authority_local_inflight_v2(node: *const NativeGeneration) -> u64 {
    node.as_ref()
        .map_or(u64::MAX, |node| node.local_inflight.load(SeqCst))
}
macro_rules! release_handle {
    ($name:ident, $ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(handle: *const $ty) {
            if !handle.is_null() {
                drop(Arc::from_raw(handle));
            }
        }
    };
}
release_handle!(mesh_authority_domain_release_v2, NativeDomain);
release_handle!(mesh_authority_generation_release_v2, NativeGeneration);
release_handle!(mesh_authority_release_v2, NativeAuthority);

pub type GeneratedExport = unsafe extern "C" fn(*const u8, u64, *mut MeshLibraryBytes) -> i32;
/// Serialized unit1 entry. `export` must be the exact generated descriptor
/// selected and narrowed by the trusted native receiving adapter. This is NOT
/// an untrusted dispatcher or the later scheduler-owned submit API.
#[no_mangle]
pub unsafe extern "C" fn mesh_library_call_scoped_v2(
    authority: *const NativeAuthority,
    export: Option<GeneratedExport>,
    input: *const u8,
    input_len: u64,
    output: *mut MeshLibraryBytes,
) -> i32 {
    if output.is_null() {
        return 1;
    }
    output.write(MeshLibraryBytes::default());
    let Some(export) = export else {
        return 1;
    };
    if authority.is_null() || (input.is_null() && input_len != 0) {
        return 1;
    }
    if current().is_some() {
        return 3;
    }
    Arc::increment_strong_count(authority);
    let original = Arc::from_raw(authority);
    if !original.live() {
        return REFUSED;
    }
    let Ok(id) = NEXT_INVOCATION.fetch_update(SeqCst, SeqCst, |n| n.checked_add(1)) else {
        return REFUSED;
    };
    let _scope = enter(Some(Arc::new(InvocationContext {
        original,
        id,
        sequence: AtomicU64::new(0),
        refused: AtomicBool::new(false),
        transactions: AtomicU64::new(0),
    })));
    export(input, input_len, output)
}
