use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

const WAIT: Duration = Duration::from_secs(5);
// These tests require a notification. The fallback probe cannot wake them
// within WAIT.
const PROBE: Duration = Duration::from_secs(15);

fn take<S>(pool: &SessionPool<S>) -> Checkout<'_, S> {
    pool.checkout(Instant::now() + WAIT, Duration::ZERO, PROBE, || true)
        .unwrap()
}

fn wait_until_parked<S>(pool: &SessionPool<S>) {
    let deadline = Instant::now() + WAIT;
    while pool.state.lock().unwrap().waiters == 0 {
        assert!(Instant::now() < deadline, "checkout never waited");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn effect_pool_gives_two_exclusive_checkouts_and_reuses_the_exact_returned_session() {
    let pool = SessionPool::new(vec![Box::new(1), Box::new(2)]);
    let mut first = take(&pool);
    let mut second = take(&pool);
    let address = std::ptr::from_ref(&**first.session());
    assert_ne!(address, std::ptr::from_ref(&**second.session()));
    assert!(!pool.exhausted_permanently());
    assert!(matches!(
        pool.checkout(
            Instant::now() + Duration::from_millis(20),
            Duration::ZERO,
            PROBE,
            || true
        ),
        Err(CheckoutError::Unavailable)
    ));
    first.reuse();
    let mut again = take(&pool);
    assert_eq!(std::ptr::from_ref(&**again.session()), address);
    assert_eq!(**again.session(), 2);
    assert_eq!(**second.session(), 1);
}

#[test]
fn receipt_reserve_ends_contention_without_blocking_an_idle_checkout() {
    let pool = SessionPool::new(vec![()]);
    let busy = take(&pool);
    let probes = std::cell::Cell::new(0);
    assert!(matches!(
        pool.checkout(Instant::now() + WAIT, WAIT, PROBE, || {
            probes.set(probes.get() + 1);
            true
        }),
        Err(CheckoutError::Unavailable)
    ));
    assert_eq!(probes.get(), 1);
    busy.reuse();
    assert!(pool
        .checkout(Instant::now() + WAIT, WAIT, PROBE, || true)
        .is_ok());
}

#[test]
fn effect_pool_release_wakes_a_waiter_and_only_returns_the_reset_lane() {
    let pool = SessionPool::new(vec![true, false]);
    let mut session = take(&pool);
    let uncertain = take(&pool);
    thread::scope(|scope| {
        let (done, result) = mpsc::channel();
        let pool = &pool;
        scope.spawn(move || {
            let mut checkout = pool
                .checkout(Instant::now() + PROBE, Duration::ZERO, PROBE, || true)
                .unwrap();
            done.send(*checkout.session()).unwrap();
            checkout.reuse();
        });
        wait_until_parked(pool);
        drop(uncertain);
        assert!(!pool.exhausted_permanently());
        // Mark the exact healthy checkout idle before returning it.
        *session.session() = true;
        session.reuse();
        assert!(result.recv_timeout(WAIT).unwrap());
    });
    let mut checkout = take(&pool);
    assert!(*checkout.session());
    drop(checkout);
    assert!(pool.exhausted_permanently());
}

// Keep one checkout outstanding until the waiter is parked. The caller
// wakes it via the real pool methods.
fn check_waiter_error(
    pool: &SessionPool<()>,
    wake: impl FnOnce(),
    live: impl Fn() -> bool + Send,
    expected: CheckoutError,
) {
    thread::scope(|scope| {
        let (done, result) = mpsc::channel();
        scope.spawn(move || {
            let result = pool.checkout(Instant::now() + PROBE, Duration::ZERO, PROBE, live);
            let Err(error) = result else {
                panic!("waiter received a session");
            };
            done.send(error).unwrap();
        });
        wait_until_parked(pool);
        wake();
        assert_eq!(result.recv_timeout(WAIT).unwrap(), expected);
    });
}

#[test]
fn effect_pool_final_retirement_wakes_waiters() {
    let pool = SessionPool::new(vec![()]);
    let uncertain = take(&pool);
    check_waiter_error(
        &pool,
        || drop(uncertain),
        || true,
        CheckoutError::Unavailable,
    );
    assert!(pool.exhausted_permanently());
}

#[test]
fn final_retirement_orders_launch_admission_without_reserving_a_checkout() {
    let pool = SessionPool::new(vec![()]);
    let final_lane = take(&pool);
    // A busy lane can still be reset, so the runner can admit a launch.
    assert!(!pool.exhausted_permanently());
    drop(final_lane);

    // A worker admitted above can spawn late, but it and any other active
    // worker can no longer acquire a session. Later launches are denied.
    assert!(pool.exhausted_permanently());
    assert!(matches!(
        pool.checkout(Instant::now() + WAIT, Duration::ZERO, PROBE, || true),
        Err(CheckoutError::Unavailable)
    ));
}

#[test]
fn effect_pool_close_wakes_waiters_and_discards_a_late_return() {
    let pool = SessionPool::new(vec![()]);
    let checked_out = take(&pool);
    check_waiter_error(&pool, || pool.close(), || true, CheckoutError::Unavailable);
    checked_out.reuse();
    assert!(pool.exhausted_permanently());
    assert!(matches!(
        pool.checkout(Instant::now() + WAIT, Duration::ZERO, PROBE, || true),
        Err(CheckoutError::Unavailable)
    ));
}

#[test]
fn effect_pool_cancellation_wakes_without_releasing_the_busy_lane() {
    let pool = SessionPool::new(vec![()]);
    let first = take(&pool);
    let live = AtomicBool::new(true);
    check_waiter_error(
        &pool,
        || {
            live.store(false, Ordering::Release);
            pool.interrupt_waiters();
        },
        || live.load(Ordering::Acquire),
        CheckoutError::Cancelled,
    );
    assert!(!pool.exhausted_permanently());
    first.reuse();
    assert!(matches!(
        pool.checkout(Instant::now() + WAIT, Duration::ZERO, PROBE, || false),
        Err(CheckoutError::Cancelled)
    ));
    take(&pool).reuse();
}

#[test]
fn effect_pool_interrupt_cannot_be_lost_between_probe_and_wait() {
    let pool = SessionPool::new(vec![()]);
    let _first = take(&pool);
    let live = AtomicBool::new(true);
    thread::scope(|scope| {
        let (probing, probed) = mpsc::sync_channel(0);
        let (resume, resumed) = mpsc::sync_channel(0);
        let (done, result) = mpsc::channel();
        let pool = &pool;
        let live = &live;
        scope.spawn(move || {
            let first_probe = AtomicBool::new(true);
            let result = pool.checkout(Instant::now() + PROBE, Duration::ZERO, PROBE, || {
                let value = live.load(Ordering::Acquire);
                if first_probe.swap(false, Ordering::Relaxed) {
                    probing.send(()).unwrap();
                    resumed.recv_timeout(WAIT).unwrap();
                }
                value
            });
            done.send(matches!(result, Err(CheckoutError::Cancelled)))
                .unwrap();
        });
        probed.recv_timeout(WAIT).unwrap();
        live.store(false, Ordering::Release);
        pool.interrupt_waiters();
        resume.send(()).unwrap();
        assert!(result.recv_timeout(WAIT).unwrap());
    });
}
