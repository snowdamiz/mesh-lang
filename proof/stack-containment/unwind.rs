use std::sync::atomic::{AtomicUsize, Ordering};

static LIMIT: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static ENTERED: AtomicUsize = AtomicUsize::new(0);
static FAILED_AT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn pthread_self() -> usize;
    fn pthread_get_stackaddr_np(thread: usize) -> *mut u8;
    fn pthread_get_stacksize_np(thread: usize) -> usize;
}
unsafe extern "C-unwind" {
    fn checked_direct(indirect: i64);
}

struct Exhausted;

#[no_mangle]
pub extern "C-unwind" fn fail_stack(sp: usize) -> ! {
    FAILED_AT.store(sp, Ordering::Relaxed);
    std::panic::resume_unwind(Box::new(Exhausted));
}

#[no_mangle]
pub extern "C" fn stack_limit() -> usize { LIMIT.load(Ordering::Relaxed) }

struct DropMarker;
impl Drop for DropMarker {
    fn drop(&mut self) { DROPPED.fetch_add(1, Ordering::Relaxed); }
}

#[no_mangle]
pub extern "C-unwind" fn runtime_bridge(indirect: i64) {
    let _marker = DropMarker;
    ENTERED.fetch_add(1, Ordering::Relaxed);
    unsafe { checked_direct(indirect) };
}

fn main() {
    for indirect in [0, 1] {
        std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let tid = unsafe { pthread_self() };
            let low = unsafe {
                pthread_get_stackaddr_np(tid) as usize - pthread_get_stacksize_np(tid)
            };
            LIMIT.store(low + 64 * 1024, Ordering::Relaxed);
            ENTERED.store(0, Ordering::Relaxed);
            DROPPED.store(0, Ordering::Relaxed);
            let result = std::panic::catch_unwind(|| unsafe { checked_direct(indirect) });
            assert!(result.unwrap_err().is::<Exhausted>());
            let entered = ENTERED.load(Ordering::Relaxed);
            let dropped = DROPPED.load(Ordering::Relaxed);
            assert_eq!(entered, dropped);
            let slack = FAILED_AT.load(Ordering::Relaxed) - low;
            assert!(slack > 0);
            println!("indirect={indirect} caught=1 entered={entered} dropped={dropped} failure_slack={slack}");
        }).unwrap().join().unwrap();
    }
    println!("process_after_unwinds=alive");
}
