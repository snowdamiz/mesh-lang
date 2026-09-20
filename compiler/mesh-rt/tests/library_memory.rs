use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use mesh_rt::bytes::MeshBytes;
use mesh_rt::library::*;
use mesh_rt::string::mesh_string_new;

struct CountedAllocator;
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for CountedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: CountedAllocator = CountedAllocator;

unsafe extern "C-unwind" fn echo(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    output.write(MeshLibraryCallResult {
        tag: 0,
        _padding: [0; 7],
        value: input.cast(),
    });
}

unsafe extern "C-unwind" fn reject(_: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
    output.write(MeshLibraryCallResult {
        tag: 1,
        _padding: [0; 7],
        value: mesh_string_new(b"rejected".as_ptr(), 8).cast(),
    });
}

unsafe extern "C-unwind" fn fail(_: *mut MeshBytes, _: *mut MeshLibraryCallResult) {
    panic!("contained library failure");
}

static READ: AtomicBool = AtomicBool::new(false);
static VALUE: AtomicU8 = AtomicU8::new(0);

extern "C" fn read_after_return(input: *const u8) {
    while !READ.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(1));
    }
    VALUE.store(unsafe { *input.add(size_of::<u64>()) }, Ordering::Release);
}

unsafe extern "C-unwind" fn spawn_reader(
    input: *mut MeshBytes,
    output: *mut MeshLibraryCallResult,
) {
    mesh_rt::mesh_actor_spawn(read_after_return as *const u8, input.cast(), 8, 1);
    echo(input, output);
}

#[test]
fn repeated_library_calls_release_temporary_memory_on_success_error_and_panic() {
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    // Match hosts that initialize on the UI thread and invoke on a worker.
    std::thread::spawn(|| {
        let request = vec![0x5a; 64 * 1024];
        let cases: [(MeshLibraryEntrypoint, i32, &[u8]); 3] = [
            (echo, MESH_LIBRARY_OK, &request),
            (reject, MESH_LIBRARY_ERR_APPLICATION, b"rejected"),
            (fail, MESH_LIBRARY_ERR_PANIC, &[]),
        ];
        let call = || {
            for (entry, expected_status, expected_bytes) in cases {
                let mut response = MeshLibraryBytes::default();
                assert_eq!(
                    unsafe {
                        mesh_library_invoke(
                            entry,
                            request.as_ptr(),
                            request.len() as u64,
                            &mut response,
                        )
                    },
                    expected_status
                );
                if expected_bytes.is_empty() {
                    assert!(response.data.is_null());
                } else {
                    assert_eq!(
                        unsafe { std::slice::from_raw_parts(response.data, response.len as usize) },
                        expected_bytes
                    );
                }
                mesh_library_free_returned_bytes(&mut response);
            }
        };
        for _ in 0..4 {
            call();
        }
        let baseline = LIVE_BYTES.load(Ordering::Relaxed);
        for _ in 0..100 {
            call();
        }
        let growth = LIVE_BYTES.load(Ordering::Relaxed) - baseline;
        assert!(
            growth < 1024 * 1024,
            "completed calls retained {growth} bytes"
        );
        assert_eq!(mesh_rt::actor::stack::get_current_pid(), None);
    })
    .join()
    .unwrap();
    // A spawned actor may still borrow the call's managed input after return.
    let request = vec![0x5a; 1024 * 1024];
    let previous_pid = mesh_rt::actor::stack::get_current_pid();
    let baseline = LIVE_BYTES.load(Ordering::Relaxed);
    let mut response = MeshLibraryBytes::default();
    assert_eq!(
        unsafe {
            mesh_library_invoke(
                spawn_reader,
                request.as_ptr(),
                request.len() as u64,
                &mut response,
            )
        },
        MESH_LIBRARY_OK
    );
    mesh_library_free_returned_bytes(&mut response);
    assert_eq!(mesh_rt::actor::stack::get_current_pid(), previous_pid);
    // Check ownership before allowing the actor to dereference the input.
    assert!(
        LIVE_BYTES.load(Ordering::Relaxed) - baseline >= request.len() as isize,
        "spawned actor's argument was freed before it finished"
    );
    READ.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (VALUE.load(Ordering::Acquire) == 0
        || LIVE_BYTES.load(Ordering::Relaxed) - baseline > 256 * 1024)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(VALUE.load(Ordering::Acquire), 0x5a);
    assert!(
        LIVE_BYTES.load(Ordering::Relaxed) - baseline < 256 * 1024,
        "completed actor retained the call heap"
    );
    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);
    let mut response = MeshLibraryBytes::default();
    assert_eq!(
        unsafe { mesh_library_invoke(echo, ptr::null(), 0, &mut response) },
        MESH_LIBRARY_ERR_NOT_INITIALIZED
    );
}
