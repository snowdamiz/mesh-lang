//! In-call GC, native thread ownership and synchronous coroutine compatibility.
use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use mesh_rt::bytes::{mesh_bytes_new, MeshBytes};
use mesh_rt::library::*;

struct Counted;
static LIVE: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;
static GROWTH: AtomicIsize = AtomicIsize::new(0);
static RELEASE: AtomicBool = AtomicBool::new(false);
static READ: AtomicU8 = AtomicU8::new(0);
static COROUTINE_DONE: AtomicBool = AtomicBool::new(false);

#[inline(never)]
fn garbage() {
    static BYTES: [u8; 65536] = [0x59; 65536];
    std::hint::black_box(mesh_bytes_new(BYTES.as_ptr(), BYTES.len() as u64));
}

unsafe extern "C-unwind" fn echo(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    output.write(MeshLibraryCallResult {
        tag: 0,
        _padding: [0; 7],
        value: input.cast(),
    });
}
unsafe extern "C-unwind" fn pressure(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    let before = LIVE.load(Ordering::Relaxed);
    for _ in 0..256 {
        garbage();
        mesh_rt::mesh_reduction_check();
    }
    GROWTH.store(LIVE.load(Ordering::Relaxed) - before, Ordering::Relaxed);
    match *input.cast::<u8>().add(8) {
        1 => output.write(MeshLibraryCallResult {
            tag: 1,
            _padding: [0; 7],
            value: mesh_rt::string::mesh_string_new(b"expected".as_ptr(), 8).cast(),
        }),
        2 => panic!("contained pressure failure"),
        _ => echo(input, output),
    }
}
extern "C" fn reader(args: *const u8) {
    let input = unsafe { Box::from_raw(args as *mut [u64; 1]) }[0] as *const u8;
    while !RELEASE.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(1));
    }
    READ.store(unsafe { *input.add(8) }, Ordering::Release);
}
unsafe extern "C-unwind" fn lend_then_collect(
    input: *mut MeshBytes,
    output: *mut MeshLibraryCallResult,
) {
    spawn_with_independent_payload();
    pressure(input, output);
}

#[inline(never)]
fn spawn_with_independent_payload() {
    static BYTES: [u8; 65536] = [0x5a; 65536];
    let payload = mesh_bytes_new(BYTES.as_ptr(), BYTES.len() as u64);
    let args = Box::into_raw(Box::new([payload as u64]));
    mesh_rt::mesh_actor_spawn(reader as *const u8, args.cast(), 8, 1);
    // This frame is gone before pressure starts. Only the child's loan names
    // this allocation, which has the same size as the later 0x59 garbage.
}
fn invoke(entry: MeshLibraryEntrypoint, mode: u8) -> i32 {
    let mut output = MeshLibraryBytes::default();
    let code = unsafe { mesh_library_invoke(entry, &mode, 1, &mut output) };
    if code == MESH_LIBRARY_OK {
        assert_eq!(output.len, 1);
        assert_eq!(unsafe { *output.data }, mode);
    } else if code == MESH_LIBRARY_ERR_APPLICATION {
        assert_eq!(
            unsafe { std::slice::from_raw_parts(output.data, output.len as usize) },
            b"expected"
        );
    } else {
        assert!(output.data.is_null());
        assert_eq!(output.len, 0);
    }
    mesh_library_free_returned_bytes(&mut output);
    code
}
extern "C" fn coroutine(_: *const u8) {
    let before = mesh_rt::actor::stack::get_current_pid();
    assert_eq!(invoke(echo, 0), MESH_LIBRARY_OK);
    assert_eq!(invoke(pressure, 0), MESH_LIBRARY_OK);
    assert_eq!(invoke(pressure, 1), MESH_LIBRARY_ERR_APPLICATION);
    assert_eq!(invoke(pressure, 2), MESH_LIBRARY_ERR_PANIC);
    assert_eq!(mesh_rt::actor::stack::get_current_pid(), before);
    // This must resume the outer coroutine with its original yielder/PID.
    mesh_rt::mesh_timer_sleep(1);
    assert_eq!(mesh_rt::actor::stack::get_current_pid(), before);
    COROUTINE_DONE.store(true, Ordering::Release);
}
fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(predicate());
}

#[test]
fn calls_collect_only_at_supported_safe_points_preserve_loans_and_restore_context() {
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    let original = mesh_rt::actor::stack::get_current_pid();
    mesh_rt::mesh_actor_spawn(coroutine as *const u8, ptr::null(), 0, 1);
    wait_until(|| COROUTINE_DONE.load(Ordering::Acquire));
    assert_eq!(mesh_rt::actor::stack::get_current_pid(), original);
    // Initialize on one native thread, invoke from another with no prior PID.
    std::thread::spawn(|| {
        assert!(mesh_rt::actor::stack::get_current_pid().is_none());
        for (mode, expected) in [
            (0, MESH_LIBRARY_OK),
            (1, MESH_LIBRARY_ERR_APPLICATION),
            (2, MESH_LIBRARY_ERR_PANIC),
        ] {
            assert_eq!(invoke(pressure, mode), expected);
            assert!(
                GROWTH.load(Ordering::Relaxed) < 8 * 1024 * 1024,
                "discarded in-call allocations were retained"
            );
            assert!(mesh_rt::actor::stack::get_current_pid().is_none());
        }
    })
    .join()
    .unwrap();
    assert_eq!(invoke(lend_then_collect, 0x33), MESH_LIBRARY_OK);
    assert!(GROWTH.load(Ordering::Relaxed) < 8 * 1024 * 1024);
    RELEASE.store(true, Ordering::Release);
    wait_until(|| READ.load(Ordering::Acquire) == 0x5a);
    assert_eq!(mesh_rt::actor::stack::get_current_pid(), original);
    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);
}
